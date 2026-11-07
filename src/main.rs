//! Reports what Godot's GDScript compiler and resource files say is wrong with a
//! project, without opening the editor or running the project's code.
//!
//! The analysis happens in `checker.gd`, inside a headless Godot. This program
//! prepares that run and turns its results into stable output.

use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

const CHECKER_SOURCE: &str = include_str!("checker.gd");
const CLASS_CACHE: &str = ".godot/global_script_class_cache.cfg";
const SCHEMA_VERSION: u32 = 1;
const EXIT_PROBLEMS: u8 = 1;
const EXIT_TOOL_FAILURE: u8 = 2;
const MINIMUM_GODOT: (u32, u32) = (4, 5);
/// Marks an override.cfg, or the part of one, that this tool wrote, so that one
/// left behind by a killed run is cleaned up by the next.
const OVERRIDE_MARKER: &str = "; gdtypecheck temporary settings, safe to delete";
const UNSAFE_ACCESS_SETTINGS: [&str; 2] = [
    "gdscript/warnings/unsafe_method_access",
    "gdscript/warnings/unsafe_property_access",
];

#[derive(Debug, Parser)]
#[command(
    name = "gdtypecheck",
    version,
    about = "Report GDScript and shader compile errors and broken references in a Godot project",
    after_help = "Exit codes: 0 no problems, 1 problems found, 2 the check could not run."
)]
struct Cli {
    /// Godot project to check.
    #[arg(long, default_value = ".")]
    project_root: PathBuf,
    /// Godot 4.5+ executable. Defaults to $GODOT_EXE, then godot on the PATH.
    #[arg(long)]
    godot_exe: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Also fail on method and property accesses the compiler cannot verify,
    /// such as a method that does not exist on a typed variable.
    #[arg(long)]
    strict: bool,
    /// Do not report warnings.
    #[arg(long, conflicts_with = "warnings_as_errors")]
    no_warnings: bool,
    /// Exit with 1 when there are warnings.
    #[arg(long)]
    warnings_as_errors: bool,
    /// Seconds to wait for each Godot run.
    #[arg(long, default_value_t = 300)]
    timeout: u64,
    /// Files or folders to check, relative to the project root.
    #[arg(value_name = "TARGET", default_value = ".")]
    targets: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
enum Format {
    Text,
    Json,
    /// SARIF 2.1.0, for GitHub code scanning and similar tools.
    Sarif,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct Diagnostic {
    file: String,
    line: u32,
    severity: Severity,
    rule: String,
    message: String,
}

#[derive(Debug, Deserialize)]
struct PassResult {
    files_checked: usize,
    diagnostics: Vec<Diagnostic>,
    warning_settings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Report<'a> {
    schema_version: u32,
    tool_version: &'a str,
    godot_version: &'a str,
    ok: bool,
    files_checked: usize,
    errors: usize,
    warnings: usize,
    duration_ms: u128,
    diagnostics: &'a [Diagnostic],
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(EXIT_PROBLEMS),
        Err(error) => {
            if cli.format == Format::Json {
                let report = serde_json::json!({
                    "schema_version": SCHEMA_VERSION,
                    "tool_version": env!("CARGO_PKG_VERSION"),
                    "ok": false,
                    "tool_error": format!("{error:#}"),
                });
                println!("{report}");
            } else {
                eprintln!("gdtypecheck: error: {error:#}");
            }
            ExitCode::from(EXIT_TOOL_FAILURE)
        }
    }
}

/// Returns whether the project passed.
fn run(cli: &Cli) -> Result<bool> {
    let started = Instant::now();
    let root = project_root(&cli.project_root)?;
    let godot = resolve_godot(cli.godot_exe.as_deref())?;
    let godot_version = godot_version(&godot)?;
    let targets = resolve_targets(&root, &cli.targets)?;
    let timeout = Duration::from_secs(cli.timeout);
    remove_leftover_override(&root)?;
    refresh_class_cache_if_stale(&godot, &root, timeout)?;

    let workspace = Workspace::create()?;
    let whole_project = targets == ["res://"];
    let first = workspace.run_pass(&godot, &root, &targets, whole_project, "errors", timeout)?;
    let mut diagnostics = first.diagnostics;

    if cli.strict || !cli.no_warnings {
        let mut settings = first.warning_settings;
        settings.extend(UNSAFE_ACCESS_SETTINGS.map(String::from));
        let override_file = OverrideFile::write(&root, &settings)?;
        let second = workspace.run_pass(&godot, &root, &targets, false, "warnings", timeout);
        override_file.restore()?;
        let warnings = select_warnings(second?.diagnostics, cli);
        // What the project already treats as an error shows up in both passes.
        let is_new = |warning: &Diagnostic| {
            !diagnostics
                .iter()
                .any(|error| error.file == warning.file && error.line == warning.line)
        };
        let warnings: Vec<Diagnostic> = warnings.into_iter().filter(is_new).collect();
        diagnostics.extend(warnings);
    }

    for diagnostic in &mut diagnostics {
        diagnostic.file = diagnostic
            .file
            .strip_prefix("res://")
            .unwrap_or(&diagnostic.file)
            .to_string();
    }
    diagnostics.sort();
    diagnostics.dedup();

    let errors = count(&diagnostics, Severity::Error);
    let warnings = count(&diagnostics, Severity::Warning);
    let ok = errors == 0 && (warnings == 0 || !cli.warnings_as_errors);
    let report = Report {
        schema_version: SCHEMA_VERSION,
        tool_version: env!("CARGO_PKG_VERSION"),
        godot_version: &godot_version,
        ok,
        files_checked: first.files_checked,
        errors,
        warnings,
        duration_ms: started.elapsed().as_millis(),
        diagnostics: &diagnostics,
    };
    match cli.format {
        Format::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        Format::Sarif => println!("{}", serde_json::to_string_pretty(&sarif_log(&report))?),
        Format::Text => print_text(&report),
    }
    Ok(ok)
}

fn count(diagnostics: &[Diagnostic], severity: Severity) -> usize {
    diagnostics
        .iter()
        .filter(|d| d.severity == severity)
        .count()
}

/// Applies the command line's choices to what the warnings pass found.
fn select_warnings(found: Vec<Diagnostic>, cli: &Cli) -> Vec<Diagnostic> {
    found
        .into_iter()
        .filter_map(|mut diagnostic| {
            if diagnostic.rule == "unsafe-access" {
                if !cli.strict {
                    return None;
                }
                diagnostic.severity = Severity::Error;
            } else if cli.no_warnings {
                return None;
            }
            Some(diagnostic)
        })
        .collect()
}

/// One rule per kind of finding, so that code scanning can group them.
const SARIF_RULES: [(&str, &str); 5] = [
    ("gdscript", "GDScript compile error"),
    ("shader", "Shader compile error"),
    (
        "missing-resource",
        "Reference to a file that does not exist",
    ),
    ("gdscript-warning", "GDScript warning"),
    ("unsafe-access", "Unknown member of a project class"),
];

fn sarif_log(report: &Report) -> serde_json::Value {
    let rules: Vec<serde_json::Value> = SARIF_RULES
        .iter()
        .map(|(id, description)| {
            serde_json::json!({ "id": id, "shortDescription": { "text": description } })
        })
        .collect();
    let results: Vec<serde_json::Value> = report.diagnostics.iter().map(sarif_result).collect();
    serde_json::json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "gdtypecheck",
                    "version": report.tool_version,
                    "informationUri": env!("CARGO_PKG_REPOSITORY"),
                    "rules": rules,
                }
            },
            "originalUriBaseIds": { "PROJECTROOT": { "description": { "text": "The Godot project root" } } },
            "results": results,
        }]
    })
}

fn sarif_result(diagnostic: &Diagnostic) -> serde_json::Value {
    let mut location = serde_json::json!({
        "artifactLocation": { "uri": diagnostic.file, "uriBaseId": "PROJECTROOT" }
    });
    if diagnostic.line > 0 {
        location["region"] = serde_json::json!({ "startLine": diagnostic.line });
    }
    let level = match diagnostic.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    serde_json::json!({
        "ruleId": diagnostic.rule,
        "level": level,
        "message": { "text": diagnostic.message },
        "locations": [{ "physicalLocation": location }],
    })
}

fn print_text(report: &Report) {
    for d in report.diagnostics {
        let severity = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        if d.line > 0 {
            println!(
                "{}:{}: {severity}[{}]: {}",
                d.file, d.line, d.rule, d.message
            );
        } else {
            println!("{}: {severity}[{}]: {}", d.file, d.rule, d.message);
        }
    }
    let files = report.files_checked;
    if report.errors == 0 && report.warnings == 0 {
        println!("gdtypecheck: ok ({files} scripts)");
    } else {
        println!(
            "gdtypecheck: {} error(s), {} warning(s) ({files} scripts)",
            report.errors, report.warnings
        );
    }
}

fn project_root(path: &Path) -> Result<PathBuf> {
    let root = fs::canonicalize(path)
        .with_context(|| format!("invalid project root {}", path.display()))?;
    let root = without_verbatim_prefix(root);
    if !root.join("project.godot").is_file() {
        bail!(
            "{} is not a Godot project: no project.godot",
            root.display()
        );
    }
    Ok(root)
}

/// Windows canonicalization yields `\\?\C:\...`, which is noise in messages.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(stripped) if !stripped.starts_with("UNC") => PathBuf::from(stripped),
        _ => path,
    }
}

fn resolve_godot(cli_override: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = cli_override {
        return which::which(path)
            .with_context(|| format!("Godot not found at {}", path.display()));
    }
    if let Some(path) = env::var_os("GODOT_EXE").filter(|value| !value.is_empty()) {
        return which::which(&path).with_context(|| {
            format!(
                "GODOT_EXE is not executable: {}",
                PathBuf::from(&path).display()
            )
        });
    }
    for name in ["godot", "godot4", "Godot"] {
        if let Ok(path) = which::which(name) {
            return Ok(path);
        }
    }
    for path in [
        r"C:\Program Files\Godot\Godot.exe",
        "/Applications/Godot.app/Contents/MacOS/Godot",
    ] {
        if Path::new(path).is_file() {
            return Ok(PathBuf::from(path));
        }
    }
    Err(anyhow!(
        "could not find Godot; pass --godot-exe or set GODOT_EXE"
    ))
}

fn godot_version(godot: &Path) -> Result<String> {
    let output = Command::new(godot)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run {}", godot.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout
        .lines()
        .rev()
        .find(|line| line.starts_with(|c: char| c.is_ascii_digit()))
        .ok_or_else(|| anyhow!("{} did not report a Godot version", godot.display()))?
        .trim()
        .to_string();
    if !is_supported(&version) {
        bail!(
            "Godot {version} is not supported: {}.{} or later is required",
            MINIMUM_GODOT.0,
            MINIMUM_GODOT.1
        );
    }
    Ok(version)
}

fn is_supported(version: &str) -> bool {
    let mut parts = version
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0)) >= MINIMUM_GODOT
}

/// Turns targets into `res://` paths. Checking the root means the whole project.
fn resolve_targets(root: &Path, targets: &[PathBuf]) -> Result<Vec<String>> {
    let mut resolved = BTreeSet::new();
    for target in targets {
        let path = if target.is_absolute() {
            target.clone()
        } else {
            root.join(target)
        };
        let path = fs::canonicalize(&path)
            .with_context(|| format!("target does not exist: {}", target.display()))?;
        let relative = without_verbatim_prefix(path)
            .strip_prefix(root)
            .map_err(|_| anyhow!("target is outside the project: {}", target.display()))?
            .display()
            .to_string()
            .replace('\\', "/");
        resolved.insert(format!("res://{relative}"));
    }
    if resolved.contains("res://") {
        return Ok(vec!["res://".to_string()]);
    }
    Ok(resolved.into_iter().collect())
}

/// Scripts Godot would see: no hidden folders, `.gdignore`d folders or nested projects.
fn project_scripts(root: &Path) -> impl Iterator<Item = PathBuf> {
    let root = root.to_path_buf();
    WalkDir::new(&root)
        .into_iter()
        .filter_entry(move |entry| {
            let path = entry.path();
            if path == root || !entry.file_type().is_dir() {
                return true;
            }
            !entry.file_name().to_string_lossy().starts_with('.')
                && !path.join(".gdignore").exists()
                && !path.join("project.godot").exists()
        })
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "gd"))
}

/// Godot resolves `class_name`s through a cache that only an import updates. On a
/// fresh checkout or after adding a class, valid code would otherwise fail.
fn refresh_class_cache_if_stale(godot: &Path, root: &Path, timeout: Duration) -> Result<()> {
    let declared: BTreeSet<String> = project_scripts(root)
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|source| declared_class(&source))
        .collect();
    let cache = fs::read_to_string(root.join(CLASS_CACHE)).ok();
    if cache
        .as_deref()
        .is_some_and(|cache| !class_cache_is_stale(cache, &declared, root))
    {
        return Ok(());
    }

    let status = run_with_timeout(
        Command::new(godot)
            .args(["--headless", "--path"])
            .arg(root)
            .arg("--import")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
        timeout,
    );
    // Godot can crash on exit after a successful import, so the exit code says
    // nothing. What matters is whether the cache is there now.
    status.context("failed to run the Godot import")?;
    if !declared.is_empty() && !root.join(CLASS_CACHE).is_file() {
        bail!("the Godot import did not produce {CLASS_CACHE}");
    }
    Ok(())
}

fn declared_class(source: &str) -> Option<String> {
    source.lines().find_map(|line| {
        let line = line.trim_start();
        let line = line.strip_prefix("@tool").map_or(line, str::trim_start);
        let name = line.strip_prefix("class_name")?;
        if !name.starts_with([' ', '\t']) {
            return None;
        }
        let name: String = name
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        (!name.is_empty()).then_some(name)
    })
}

fn class_cache_is_stale(cache: &str, declared: &BTreeSet<String>, root: &Path) -> bool {
    let cached_classes = quoted_values(cache, "\"class\": &\"");
    let has_undeclared = declared.iter().any(|class| !cached_classes.contains(class));
    let has_moved = quoted_values(cache, "\"path\": \"").iter().any(|path| {
        path.strip_prefix("res://")
            .is_some_and(|relative| !root.join(relative).exists())
    });
    has_undeclared || has_moved
}

fn quoted_values(text: &str, prefix: &str) -> BTreeSet<String> {
    text.split(prefix)
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .map(String::from)
        .collect()
}

/// Runs Godot with its output sent to a file: Godot 4.6 can crash on Windows when
/// its stderr is a pipe, and the results come through a file anyway.
fn run_with_timeout(command: &mut Command, timeout: Duration) -> Result<()> {
    let mut child = command.spawn().context("failed to start Godot")?;
    let started = Instant::now();
    while child.try_wait()?.is_none() {
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Godot did not finish within {} seconds", timeout.as_secs());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// The temporary folder holding the checker and what it exchanges with this program.
struct Workspace {
    dir: PathBuf,
    checker: PathBuf,
}

impl Workspace {
    fn create() -> Result<Self> {
        let dir = env::temp_dir().join(format!("gdtypecheck-{}", std::process::id()));
        fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
        let checker = dir.join("checker.gd");
        fs::write(&checker, CHECKER_SOURCE)?;
        Ok(Self { dir, checker })
    }

    fn run_pass(
        &self,
        godot: &Path,
        root: &Path,
        targets: &[String],
        whole_project: bool,
        pass: &str,
        timeout: Duration,
    ) -> Result<PassResult> {
        let result_path = self.dir.join(format!("{pass}.json"));
        let log_path = self.dir.join(format!("{pass}.log"));
        let request_path = self.dir.join(format!("{pass}.request.json"));
        let request = serde_json::json!({
            "targets": targets,
            "pass": pass,
            "whole_project": whole_project,
            "output": slash_path(&result_path),
        });
        fs::write(&request_path, request.to_string())?;

        let log = fs::File::create(&log_path)?;
        run_with_timeout(
            Command::new(godot)
                .args(["--headless", "--path"])
                .arg(root)
                .arg("--script")
                .arg(slash_path(&self.checker))
                .arg("--")
                .arg(slash_path(&request_path))
                .stdin(Stdio::null())
                .stdout(log.try_clone()?)
                .stderr(log),
            timeout,
        )?;

        // The checker kills its own process once done, so only the result counts.
        let text = fs::read_to_string(&result_path).map_err(|_| {
            let log = fs::read_to_string(&log_path).unwrap_or_default();
            let tail: Vec<&str> = log.lines().rev().take(15).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            anyhow!(
                "Godot exited without producing results:\n{}",
                tail.join("\n")
            )
        })?;
        serde_json::from_str(&text).context("the checker produced invalid results")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn slash_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// A temporary override.cfg, the only way to change warning levels before Godot
/// reads them. Restored on drop so that an error cannot leave it behind.
struct OverrideFile {
    path: PathBuf,
    original: Option<String>,
    restored: bool,
}

impl OverrideFile {
    fn write(root: &Path, warning_settings: &[String]) -> Result<Self> {
        let path = root.join("override.cfg");
        let original = fs::read_to_string(&path).ok();
        let mut contents = original.clone().unwrap_or_default();
        contents.push_str(&format!("\n{OVERRIDE_MARKER}\n[debug]\n"));
        for setting in warning_settings {
            contents.push_str(&format!("{setting}=2\n"));
        }
        fs::write(&path, contents)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(Self {
            path,
            original,
            restored: false,
        })
    }

    fn restore(mut self) -> Result<()> {
        self.restored = true;
        self.put_back()
    }

    fn put_back(&self) -> Result<()> {
        match &self.original {
            Some(original) => fs::write(&self.path, original)?,
            None => fs::remove_file(&self.path)?,
        }
        Ok(())
    }
}

impl Drop for OverrideFile {
    fn drop(&mut self) {
        if !self.restored {
            let _ = self.put_back();
        }
    }
}

fn remove_leftover_override(root: &Path) -> Result<()> {
    let path = root.join("override.cfg");
    let Ok(contents) = fs::read_to_string(&path) else {
        return Ok(());
    };
    let Some(position) = contents.find(OVERRIDE_MARKER) else {
        return Ok(());
    };
    let original = contents[..position].trim_end();
    if original.is_empty() {
        fs::remove_file(&path)?;
    } else {
        fs::write(&path, format!("{original}\n"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_declared_classes() {
        assert_eq!(
            declared_class("class_name Foo\nextends Node"),
            Some("Foo".into())
        );
        assert_eq!(
            declared_class("@tool\nclass_name Foo_2 extends Node"),
            Some("Foo_2".into())
        );
        assert_eq!(declared_class("@tool class_name Foo"), Some("Foo".into()));
        assert_eq!(declared_class("extends Node\nvar class_name_x = 1"), None);
    }

    #[test]
    fn minimum_version_is_enforced() {
        assert!(is_supported("4.5.stable.official.abc"));
        assert!(is_supported("4.7.stable"));
        assert!(is_supported("5.0.stable"));
        assert!(!is_supported("4.4.1.stable"));
        assert!(!is_supported("3.6.stable"));
    }

    #[test]
    fn stale_cache_detection() {
        let cache = r#"list=[{ "base": &"Node", "class": &"Foo", "icon": "", "language": &"GDScript", "path": "res://definitely/not/here.gd" }]"#;
        assert_eq!(
            quoted_values(cache, "\"class\": &\""),
            BTreeSet::from(["Foo".to_string()])
        );
        let root = env::temp_dir();
        let declared = BTreeSet::from(["Foo".to_string()]);
        assert!(
            class_cache_is_stale(cache, &declared, &root),
            "a cached file has moved"
        );
        assert!(
            class_cache_is_stale("", &declared, &root),
            "a class is not cached"
        );
        assert!(!class_cache_is_stale("", &BTreeSet::new(), &root));
    }
}

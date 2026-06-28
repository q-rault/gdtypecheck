use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use globwalk::GlobWalkerBuilder;

const CHECKER_REL_PATH: &str = ".gdtypecheck/state/typecheck_checker.gd";
#[derive(Debug, Parser)]
#[command(name = "gdtypecheck", version, about = "Run a Godot-driven GDScript load/type check")]
struct Cli {
    #[arg(long, default_value = ".")]
    project_root: PathBuf,
    #[arg(long)]
    godot_exe: Option<PathBuf>,
    #[arg(value_name = "TARGET", default_value = ".")]
    targets: Vec<PathBuf>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let project_root = fs::canonicalize(&cli.project_root)
        .with_context(|| format!("invalid project root {}", cli.project_root.display()))?;
    let godot_exe = resolve_godot(cli.godot_exe.as_deref())?;
    let files = discover_gd_files(&project_root, &cli.targets)?;
    if files.is_empty() {
        return Err(anyhow!("no .gd files matched the provided targets"));
    }

    let checker_path = project_root.join(CHECKER_REL_PATH);
    if let Some(parent) = checker_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    write_checker_script(&checker_path, &project_root, &files)?;
    run_godot_checker(&godot_exe, &project_root)
}

fn resolve_godot(cli_override: Option<&Path>) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = cli_override {
        candidates.push(path.to_path_buf());
    }
    if let Some(path) = env::var_os("GODOT_EXE") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(path) = which::which("godot") {
        candidates.push(path);
    }
    if let Ok(path) = which::which("godot4") {
        candidates.push(path);
    }
    candidates.push(PathBuf::from(r"C:\Program Files\Godot\Godot.exe"));
    candidates.push(PathBuf::from(r"C:\Program Files\Godot_v4\Godot.exe"));

    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
        if candidate.parent().is_none() {
            if let Ok(resolved) = which::which(&candidate) {
                return Ok(resolved);
            }
        }
    }

    Err(anyhow!(
        "could not resolve Godot executable; set --godot-exe or GODOT_EXE"
    ))
}

fn discover_gd_files(project_root: &Path, targets: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = BTreeSet::new();

    for target in targets {
        let target = if target.is_absolute() {
            target.clone()
        } else {
            project_root.join(target)
        };
        if target.is_file() {
            if target.extension().and_then(|ext| ext.to_str()) == Some("gd") {
                files.insert(canonical_relative(project_root, &target)?);
            }
            continue;
        }

        if target.is_dir() {
            let relative_root = canonical_relative(project_root, &target)?;
            let search_root = project_root.join(&relative_root);
            let patterns = vec!["**/*.gd"];
            let walker = GlobWalkerBuilder::from_patterns(&search_root, &patterns)
                .follow_links(false)
                .build()
                .with_context(|| format!("failed to walk {}", search_root.display()))?;
            for entry in walker.filter_map(Result::ok) {
                let path = entry.path();
                if is_excluded(project_root, path) {
                    continue;
                }
                files.insert(canonical_relative(project_root, path)?);
            }
            continue;
        }

        return Err(anyhow!("target does not exist: {}", target.display()));
    }

    Ok(files.into_iter().collect())
}

fn canonical_relative(project_root: &Path, path: &Path) -> Result<PathBuf> {
    let absolute = fs::canonicalize(path)
        .with_context(|| format!("failed to resolve {}", path.display()))?;
    absolute
        .strip_prefix(project_root)
        .map(Path::to_path_buf)
        .map_err(|_| {
            anyhow!(
                "path {} is outside project root {}",
                absolute.display(),
                project_root.display()
            )
        })
}

fn is_excluded(project_root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(project_root) else {
        return true;
    };
    let normalized = relative.display().to_string().replace('\\', "/");
    normalized.starts_with("addons/")
        || normalized.starts_with(".godot/")
        || normalized.starts_with("reports/")
        || normalized.starts_with(".gdsuite/")
        || normalized.starts_with(".gdtypecheck/")
}

fn write_checker_script(checker_path: &Path, project_root: &Path, files: &[PathBuf]) -> Result<()> {
    let mut lines = vec![
        "extends SceneTree".to_string(),
        String::new(),
        "func _init():".to_string(),
        "\tvar files = [".to_string(),
    ];
    for file in files {
        let normalized = file.display().to_string().replace('\\', "/");
        lines.push(format!("\t\t\"res://{normalized}\","));
    }
    lines.extend([
        "\t]".to_string(),
        "\tvar errors = []".to_string(),
        "\tfor file_path in files:".to_string(),
        "\t\tif load(file_path) == null:".to_string(),
        "\t\t\terrors.append(file_path)".to_string(),
        "\tif errors.size() > 0:".to_string(),
        "\t\tfor file_path in errors:".to_string(),
        "\t\t\tprinterr(file_path)".to_string(),
        "\t\tquit(1)".to_string(),
        "\tprint(\"gdtypecheck: ok\")".to_string(),
        "\tquit(0)".to_string(),
    ]);

    fs::write(checker_path, lines.join("\n") + "\n").with_context(|| {
        format!(
            "failed to write checker script {} inside {}",
            checker_path.display(),
            project_root.display()
        )
    })
}

fn run_godot_checker(godot_exe: &Path, project_root: &Path) -> Result<()> {
    let output = Command::new(godot_exe)
        .args([
            "--headless",
            "--path",
            &project_root.display().to_string(),
            "--script",
            &format!("res://{}", CHECKER_REL_PATH.replace('\\', "/")),
        ])
        .current_dir(project_root)
        .output()
        .with_context(|| format!("failed to execute {}", godot_exe.display()))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.trim().is_empty() {
            print!("{stdout}");
        }
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "typecheck failed with exit code {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        stdout,
        stderr
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_suite_and_vendor_dirs() {
        assert!(is_excluded(Path::new("C:/repo"), Path::new("C:/repo/addons/foo.gd")));
        assert!(is_excluded(Path::new("C:/repo"), Path::new("C:/repo/.gdsuite/state/x.gd")));
        assert!(!is_excluded(Path::new("C:/repo"), Path::new("C:/repo/client/foo.gd")));
    }

    #[test]
    fn checker_script_uses_res_paths() {
        let script_root = PathBuf::from("C:/repo");
        let temp = tempfile_dir();
        let checker = temp.join("typecheck_checker.gd");
        write_checker_script(&checker, &script_root, &[PathBuf::from("client/foo.gd")]).unwrap();
        let body = fs::read_to_string(checker).unwrap();
        assert!(body.contains("\"res://client/foo.gd\""));
    }

    fn tempfile_dir() -> PathBuf {
        let path = env::temp_dir().join("gdtypecheck-tests");
        let _ = fs::create_dir_all(&path);
        path
    }
}

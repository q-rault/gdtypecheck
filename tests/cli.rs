//! Runs gdtypecheck against real Godot projects built in temporary folders. Needs
//! `GODOT_EXE`; the tests are skipped without it, and CI always sets it.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use tempfile::TempDir;

const PLAYER: &str = "class_name Player\nextends Node\n\nvar health: int = 10\n\n\nfunc hit(amount: int) -> void:\n\thealth -= amount\n";

const EVERY_WARNING: &[&str] = &[
    "confusable_temporary_modification",
    "unassigned_variable",
    "unassigned_variable_op_assign",
    "unused_variable",
    "unused_local_constant",
    "unused_private_class_variable",
    "unused_parameter",
    "unused_signal",
    "shadowed_variable",
    "shadowed_variable_base_class",
    "shadowed_global_identifier",
    "unreachable_code",
    "unreachable_pattern",
    "standalone_expression",
    "standalone_ternary",
    "incompatible_ternary",
    "untyped_declaration",
    "inferred_declaration",
    "unsafe_property_access",
    "unsafe_method_access",
    "unsafe_cast",
    "unsafe_call_argument",
    "unsafe_void_return",
    "return_value_discarded",
    "static_called_on_instance",
    "missing_tool",
    "redundant_static_unload",
    "redundant_await",
    "missing_await",
    "assert_always_true",
    "assert_always_false",
    "integer_division",
    "narrowing_conversion",
    "int_as_enum_without_cast",
    "int_as_enum_without_match",
    "enum_variable_without_default",
    "empty_file",
    "deprecated_keyword",
    "confusable_identifier",
    "confusable_local_declaration",
    "confusable_local_usage",
    "confusable_capture_reassignment",
    "inference_on_variant",
    "native_method_override",
    "get_node_default_without_onready",
    "onready_with_export",
    "property_used_as_function",
    "constant_used_as_function",
    "function_used_as_property",
];

struct Project {
    _dir: TempDir,
    root: PathBuf,
}

impl Project {
    /// `None` when Godot is not available.
    fn new() -> Option<Self> {
        if std::env::var_os("GODOT_EXE").is_none() {
            eprintln!("GODOT_EXE is not set: skipping");
            return None;
        }
        let dir = TempDir::new().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let project = Self { _dir: dir, root };
        project.write(
            "project.godot",
            "config_version=5\n\n[application]\nconfig/name=\"t\"\n",
        );
        Some(project)
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn append_settings(&self, settings: &str) {
        let path = self.root.join("project.godot");
        let current = fs::read_to_string(&path).unwrap();
        fs::write(path, format!("{current}\n{settings}\n")).unwrap();
    }

    fn exists(&self, path: &str) -> bool {
        self.root.join(path).exists()
    }

    fn run(&self, args: &[&str]) -> Run {
        run_in(&self.root, args)
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_in(root: &Path, args: &[&str]) -> Run {
    let output: Output = Command::new(env!("CARGO_BIN_EXE_gdtypecheck"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

macro_rules! project {
    () => {
        match Project::new() {
            Some(project) => project,
            None => return,
        }
    };
}

#[track_caller]
fn assert_reports(run: &Run, expected: &str) {
    assert_eq!(
        run.code, 1,
        "expected problems\n{}\n{}",
        run.stdout, run.stderr
    );
    assert!(
        run.stdout.contains(expected),
        "expected `{expected}` in:\n{}",
        run.stdout
    );
}

#[track_caller]
fn assert_clean(run: &Run) {
    assert_eq!(
        run.code, 0,
        "expected no problems\n{}\n{}",
        run.stdout, run.stderr
    );
    assert!(run.stdout.contains("gdtypecheck: ok"), "{}", run.stdout);
}

#[test]
fn a_correct_project_passes() {
    let p = project!();
    p.write("player.gd", PLAYER);
    p.write("user.gd", "extends Node\n\n\nfunc f() -> void:\n\tvar p := Player.new()\n\tp.hit(1)\n\tp.queue_free()\n");
    assert_clean(&p.run(&[]));
}

#[test]
fn compile_errors_are_reported_with_their_location() {
    let cases = [
        ("extends Node\nfunc f(:\n", "bad.gd:2: error[gdscript]"),
        (
            "extends Node\nfunc f() -> void:\n\tvar x: int = \"text\"\n\tprint(x)\n",
            "bad.gd:3: error[gdscript]",
        ),
        (
            "extends Node\nfunc f() -> int:\n\treturn \"text\"\n",
            "bad.gd:3: error[gdscript]",
        ),
        (
            "extends Node\nfunc f() -> void:\n\tPlayer.new().hit(\"ten\")\n",
            "bad.gd:3: error[gdscript]",
        ),
        (
            "extends Node\nfunc f() -> void:\n\tPlayer.new().hit()\n",
            "bad.gd:3: error[gdscript]",
        ),
        (
            "extends Node\nvar p: NoSuchClass\n",
            "bad.gd:2: error[gdscript]",
        ),
        (
            "extends Node\nfunc f() -> void:\n\tprint(undefined_name)\n",
            "bad.gd:3: error[gdscript]",
        ),
        (
            "extends Node\nconst A = preload(\"res://nope.gd\")\n",
            "bad.gd:2: error[gdscript]",
        ),
    ];
    for (source, expected) in cases {
        let p = project!();
        p.write("player.gd", PLAYER);
        p.write("bad.gd", source);
        assert_reports(&p.run(&[]), expected);
    }
}

#[test]
fn an_error_is_reported_on_its_cause_not_on_every_dependent() {
    let p = project!();
    p.write(
        "dep.gd",
        "class_name Dep\nextends Node\nfunc f() -> int:\n\treturn \"x\"\n",
    );
    p.write(
        "user.gd",
        "extends Node\nconst D = preload(\"res://dep.gd\")\n",
    );
    let run = p.run(&[]);
    assert_reports(&run, "dep.gd:4: error[gdscript]");
    assert!(!run.stdout.contains("Failed to compile"), "{}", run.stdout);
}

#[test]
fn broken_references_are_reported() {
    let p = project!();
    p.write("load.gd", "extends Node\nfunc f() -> void:\n\tprint(load(\"res://gone.png\"))\n\t# load(\"res://only_a_comment.png\")\n");
    p.write(
        "script.tscn",
        "[gd_scene load_steps=2 format=3]\n\n[ext_resource type=\"Script\" path=\"res://gone.gd\" id=\"1\"]\n\n[node name=\"Root\" type=\"Node\"]\nscript = ExtResource(\"1\")\n",
    );
    p.write(
        "instance.tscn",
        "[gd_scene load_steps=2 format=3]\n\n[ext_resource type=\"PackedScene\" path=\"res://gone.tscn\" id=\"1\"]\n\n[node name=\"Root\" type=\"Node\"]\n\n[node name=\"Child\" parent=\".\" instance=ExtResource(\"1\")]\n",
    );
    p.append_settings("[autoload]\nBus=\"*res://gone_bus.gd\"\n");
    let run = p.run(&[]);
    assert_reports(&run, "load.gd:3: error[missing-resource]");
    assert_reports(&run, "script.tscn: error[missing-resource]");
    assert_reports(&run, "instance.tscn: error[missing-resource]");
    assert_reports(&run, "project.godot: error[missing-resource]: autoload Bus");
    assert!(!run.stdout.contains("only_a_comment"), "{}", run.stdout);
}

#[test]
fn a_moved_file_whose_uid_still_resolves_is_not_a_broken_reference() {
    let p = project!();
    p.write("moved.gd", "extends Node\n");
    p.write("moved.gd.uid", "uid://bexamplemoved\n");
    p.write(
        "scene.tscn",
        "[gd_scene load_steps=2 format=3]\n\n[ext_resource type=\"Script\" uid=\"uid://bexamplemoved\" path=\"res://old_location.gd\" id=\"1\"]\n\n[node name=\"Root\" type=\"Node\"]\nscript = ExtResource(\"1\")\n",
    );
    assert_clean(&p.run(&[]));
}

#[test]
fn every_kind_of_valid_shader_passes() {
    let p = project!();
    p.write("canvas.gdshader", "shader_type canvas_item;\nuniform vec4 tint = vec4(1.0);\nvoid fragment() { COLOR = tint; }\n");
    p.write(
        "spatial.gdshader",
        "shader_type spatial;\nrender_mode unshaded;\nvoid fragment() { ALBEDO = vec3(1.0); }\n",
    );
    p.write(
        "particles.gdshader",
        "shader_type particles;\nvoid process() { VELOCITY = vec3(1.0); }\n",
    );
    p.write(
        "sky.gdshader",
        "shader_type sky;\nvoid sky() { COLOR = vec3(1.0); }\n",
    );
    p.write(
        "fog.gdshader",
        "shader_type fog;\nvoid fog() { DENSITY = 1.0; }\n",
    );
    p.write("lib/helper.gdshaderinc", "float helper() { return 1.0; }\n");
    p.write("lib/uses.gdshader", "shader_type canvas_item;\n#include \"helper.gdshaderinc\"\nvoid fragment() { COLOR = vec4(helper()); }\n");
    assert_clean(&p.run(&[]));
}

#[test]
fn shader_errors_are_reported_with_their_location() {
    let p = project!();
    p.write(
        "bad.gdshader",
        "shader_type canvas_item;\n\nvoid fragment() {\n\tCOLOR = vec2(1.0);\n}\n",
    );
    p.write("bad.gdshaderinc", "float helper() { return nope; }\n");
    p.write("uses.gdshader", "shader_type canvas_item;\n#include \"res://bad.gdshaderinc\"\nvoid fragment() { COLOR = vec4(helper()); }\n");
    p.write("gone.gdshader", "shader_type canvas_item;\n#include \"res://gone.gdshaderinc\"\nvoid fragment() { COLOR = vec4(1.0); }\n");
    let run = p.run(&[]);
    assert_reports(&run, "bad.gdshader:4: error[shader]");
    assert_reports(&run, "bad.gdshaderinc:1: error[shader]");
    assert_reports(&run, "gone.gdshader:2: error[missing-resource]");
    assert!(!run.stdout.contains("Unknown character"), "{}", run.stdout);
}

#[test]
fn a_shader_error_is_reported_once_when_a_script_preloads_it() {
    let p = project!();
    p.write(
        "bad.gdshader",
        "shader_type canvas_item;\nvoid fragment() { COLOR = vec2(1.0); }\n",
    );
    p.write(
        "user.gd",
        "extends Node\nconst S = preload(\"res://bad.gdshader\")\n",
    );
    let run = p.run(&[]);
    assert_reports(&run, "bad.gdshader:2: error[shader]");
    assert_eq!(run.stdout.matches("error[").count(), 1, "{}", run.stdout);
}

#[test]
fn autoloads_are_understood_but_never_run() {
    let p = project!();
    p.append_settings("[autoload]\nBoot=\"*res://boot.gd\"\n");
    p.write(
        "boot.gd",
        "extends Node\nfunc _init() -> void:\n\tFileAccess.open(\"res://init_ran\", FileAccess.WRITE).store_string(\"x\")\nfunc _ready() -> void:\n\tFileAccess.open(\"res://ready_ran\", FileAccess.WRITE).store_string(\"x\")\nfunc value() -> int:\n\treturn 1\n",
    );
    p.write(
        "user.gd",
        "extends Node\nfunc f() -> int:\n\treturn Boot.value()\n",
    );
    p.write(
        "user_of_user.gd",
        "extends Node\nconst U = preload(\"res://user.gd\")\n",
    );
    assert_clean(&p.run(&[]));
    assert!(
        !p.exists("init_ran") && !p.exists("ready_ran"),
        "autoload code ran"
    );

    p.write(
        "misuse.gd",
        "extends Node\nfunc f() -> String:\n\treturn Boot.value()\n",
    );
    assert_reports(&p.run(&[]), "misuse.gd:3: error[gdscript]");
}

#[test]
fn folders_godot_ignores_are_skipped() {
    let p = project!();
    let broken = "extends Node\nfunc f() -> int:\n\treturn \"x\"\n";
    p.write("ignored/.gdignore", "");
    p.write("ignored/bad.gd", broken);
    p.write("nested/project.godot", "config_version=5\n");
    p.write("nested/bad.gd", broken);
    p.write("addons/plugin/bad.gd", broken);
    p.write(".hidden/bad.gd", broken);
    assert_clean(&p.run(&[]));
}

#[test]
fn a_fresh_checkout_and_a_new_class_need_no_manual_import() {
    let p = project!();
    p.write("player.gd", PLAYER);
    p.write("user.gd", "extends Node\nvar p: Player\n");
    assert!(!p.exists(".godot"));
    assert_clean(&p.run(&[]));

    p.write("added.gd", "class_name Added\nextends Node\n");
    p.write("user2.gd", "extends Node\nvar a: Added\n");
    assert_clean(&p.run(&[]));
}

#[test]
fn warnings_are_shown_without_failing_by_default() {
    let p = project!();
    p.write(
        "warn.gd",
        "extends Node\nfunc f() -> void:\n\tvar unused := 1\n",
    );
    let run = p.run(&[]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(
        run.stdout.contains("warn.gd:3: warning[gdscript-warning]"),
        "{}",
        run.stdout
    );

    assert_reports(&p.run(&["--warnings-as-errors"]), "warn.gd:3: warning");
    assert_clean(&p.run(&["--no-warnings"]));
}

#[test]
fn warnings_the_project_made_errors_fail_the_check() {
    let p = project!();
    p.append_settings("[debug]\ngdscript/warnings/unused_variable=2\n");
    p.write(
        "warn.gd",
        "extends Node\nfunc f() -> void:\n\tvar unused := 1\n",
    );
    assert_reports(&p.run(&["--no-warnings"]), "warn.gd:3: error[gdscript]");
}

/// Project settings apply to the checker script too, so it has to compile under the
/// strictest of them.
#[test]
fn the_strictest_project_settings_do_not_break_the_check() {
    let p = project!();
    p.write(
        "warn.gd",
        "extends Node\nfunc f() -> void:\n\tvar unused := 1\n",
    );
    let settings: String = EVERY_WARNING
        .iter()
        .map(|warning| format!("gdscript/warnings/{warning}=2\n"))
        .collect();
    p.append_settings(&format!("[debug]\n{settings}"));
    let run = p.run(&[]);
    assert_reports(&run, "warn.gd:3: error[gdscript]");
    assert!(!run.stdout.contains("checker"), "{}", run.stdout);
}

#[test]
fn strict_catches_unknown_members_of_project_classes_only() {
    let p = project!();
    p.write("player.gd", PLAYER);
    p.write(
        "bad.gd",
        "extends Node\nfunc f() -> void:\n\tvar p := Player.new()\n\tp.missing_method()\n\tprint(p.missing_prop)\n",
    );
    assert_clean(&p.run(&["--no-warnings"]));
    let run = p.run(&["--strict", "--no-warnings"]);
    assert_reports(&run, "bad.gd:4: error[unsafe-access]");
    assert_reports(&run, "bad.gd:5: error[unsafe-access]");

    let p = project!();
    p.write(
        "dynamic.gd",
        "extends Node\nfunc f() -> void:\n\tget_parent().could_be_anything()\n\tvar v: Variant = 1\n\tv.anything()\n",
    );
    assert_clean(&p.run(&["--strict", "--no-warnings"]));
}

#[test]
fn targets_limit_the_check() {
    let p = project!();
    p.write("good/ok.gd", "extends Node\n");
    p.write(
        "bad/bad.gd",
        "extends Node\nfunc f() -> int:\n\treturn \"x\"\n",
    );
    assert_clean(&p.run(&["good"]));
    assert_clean(&p.run(&["good/ok.gd"]));
    assert_reports(&p.run(&["bad"]), "bad/bad.gd:3");
}

#[test]
fn json_output_is_structured() {
    let p = project!();
    p.write("bad.gd", "extends Node\nfunc f() -> int:\n\treturn \"x\"\n");
    let run = p.run(&["--format", "json"]);
    assert_eq!(run.code, 1);
    let report: serde_json::Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["ok"], false);
    assert_eq!(report["errors"], 1);
    assert_eq!(report["files_checked"], 1);
    let diagnostic = &report["diagnostics"][0];
    assert_eq!(diagnostic["file"], "bad.gd");
    assert_eq!(diagnostic["line"], 3);
    assert_eq!(diagnostic["severity"], "error");
    assert_eq!(diagnostic["rule"], "gdscript");
}

#[test]
fn sarif_output_locates_every_finding() {
    let p = project!();
    p.write(
        "sub/bad.gd",
        "extends Node\nfunc f() -> int:\n\treturn \"x\"\n",
    );
    p.write(
        "broken.tscn",
        "[gd_scene load_steps=2 format=3]\n\n[ext_resource type=\"Script\" path=\"res://gone.gd\" id=\"1\"]\n\n[node name=\"Root\" type=\"Node\"]\nscript = ExtResource(\"1\")\n",
    );
    let run = p.run(&["--format", "sarif"]);
    assert_eq!(run.code, 1);
    let log: serde_json::Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(log["version"], "2.1.0");
    let tool_run = &log["runs"][0];
    assert_eq!(tool_run["tool"]["driver"]["name"], "gdtypecheck");
    let results = tool_run["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);

    let in_scene = &results[0]["locations"][0]["physicalLocation"];
    assert_eq!(results[0]["ruleId"], "missing-resource");
    assert_eq!(in_scene["artifactLocation"]["uri"], "broken.tscn");
    assert!(
        in_scene.get("region").is_none(),
        "a file-level finding has no line"
    );

    let in_script = &results[1]["locations"][0]["physicalLocation"];
    assert_eq!(results[1]["level"], "error");
    assert_eq!(in_script["artifactLocation"]["uri"], "sub/bad.gd");
    assert_eq!(in_script["artifactLocation"]["uriBaseId"], "PROJECTROOT");
    assert_eq!(in_script["region"]["startLine"], 3);

    let rules = tool_run["tool"]["driver"]["rules"].as_array().unwrap();
    for result in results {
        assert!(
            rules.iter().any(|rule| rule["id"] == result["ruleId"]),
            "{result}"
        );
    }
}

#[test]
fn clean_sarif_output_has_no_results() {
    let p = project!();
    p.write("ok.gd", "extends Node\n");
    let run = p.run(&["--format", "sarif"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let log: serde_json::Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(log["runs"][0]["results"].as_array().unwrap().len(), 0);
}

#[test]
fn tool_failures_exit_with_2() {
    let p = project!();
    let not_a_project = TempDir::new().unwrap();
    assert_eq!(run_in(not_a_project.path(), &[]).code, 2);
    assert_eq!(p.run(&["no/such/target"]).code, 2);
    assert_eq!(p.run(&["--godot-exe", "no-such-godot"]).code, 2);
    let run = run_in(not_a_project.path(), &["--format", "json"]);
    let report: serde_json::Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert!(report["tool_error"].is_string());
}

#[test]
fn the_project_is_left_as_it_was() {
    let p = project!();
    p.write(
        "warn.gd",
        "extends Node\nfunc f() -> void:\n\tvar unused := 1\n",
    );
    p.run(&[]);
    assert!(!p.exists("override.cfg"));

    let mine = "[display]\nwindow/size/viewport_width=640\n";
    p.write("override.cfg", mine);
    p.run(&[]);
    assert_eq!(
        fs::read_to_string(p.root.join("override.cfg")).unwrap(),
        mine
    );
}

#[test]
fn settings_left_by_a_killed_run_are_cleaned_up() {
    let p = project!();
    p.write("ok.gd", "extends Node\n");
    p.write(
        "override.cfg",
        "[display]\nwindow/size/viewport_width=640\n\n; gdtypecheck temporary settings, safe to delete\n[debug]\ngdscript/warnings/unused_variable=2\n",
    );
    assert_clean(&p.run(&[]));
    assert_eq!(
        fs::read_to_string(p.root.join("override.cfg")).unwrap(),
        "[display]\nwindow/size/viewport_width=640\n"
    );
}

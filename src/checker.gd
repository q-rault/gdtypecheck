extends SceneTree

# A project's warning settings apply to this script too, and it has to compile
# under the strictest of them.
@warning_ignore_start(
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
)

# Loads a project's scripts in a headless Godot and reports what the GDScript
# compiler and the project's resource files say is wrong. See README.md.
#
# load() cannot be used to detect errors: it returns a non-null script even when
# the script fails to parse. Errors are collected from Godot's logger instead,
# which also provides their file and line.
#
# Everything happens in _init, before Godot creates the project's autoloads, and
# the process is then killed so that they are never created. That keeps the
# project's own code from running during a check. Static variable initializers
# of the loaded scripts are the one exception, as they run when a script loads.

const WARNINGS_PREFIX := "debug/gdscript/warnings/"
const WARNING_SUFFIX := " (Warning treated as error.)"
const LEVEL_WARN := 1
const SKIPPED_ROOT_DIRS := ["addons"]
const RESOURCE_EXTENSIONS := ["tscn", "tres", "scn", "res"]
const UNSAFE_MARKER := "may be present on a subtype"


class ErrorCollector extends Logger:
	var _mutex := Mutex.new()
	var entries: Array[Dictionary] = []

	func _log_error(
		_function: String,
		file: String,
		line: int,
		code: String,
		rationale: String,
		_editor_notify: bool,
		error_type: int,
		_script_backtrace: Array[ScriptBacktrace]
	) -> void:
		if error_type != ERROR_TYPE_SCRIPT:
			return
		_mutex.lock()
		entries.append({"file": file, "line": line, "message": rationale if rationale != "" else code})
		_mutex.unlock()


var _diagnostics: Array[Dictionary] = []
var _seen := {}


func _init() -> void:
	var request_path: String = OS.get_cmdline_user_args()[0]
	var request: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(request_path))

	var scripts: Array[String] = []
	var resources: Array[String] = []
	for target: String in request["targets"]:
		_collect_files(target, scripts, resources)

	var report_warnings: bool = request["pass"] == "warnings"

	var collector := ErrorCollector.new()
	OS.add_logger(collector)
	var unloadable := _load_scripts(scripts)
	OS.remove_logger(collector)
	_report_script_errors(collector.entries, unloadable, report_warnings)

	if not report_warnings:
		_check_string_references(scripts)
		_check_resources(resources)
		if request["whole_project"]:
			_check_project_settings()

	var output := FileAccess.open(request["output"], FileAccess.WRITE)
	output.store_string(JSON.stringify({
		"files_checked": scripts.size(),
		"diagnostics": _diagnostics,
		"warning_settings": _warning_settings(),
	}))
	output.close()
	OS.kill(OS.get_process_id())


func _collect_files(target: String, scripts: Array[String], resources: Array[String]) -> void:
	if FileAccess.file_exists(target):
		if target.get_extension() == "gd":
			scripts.append(target)
		elif target.get_extension() in RESOURCE_EXTENSIONS:
			resources.append(target)
		return
	for dir_name: String in DirAccess.get_directories_at(target):
		var path := target.path_join(dir_name)
		if dir_name.begins_with(".") or (target == "res://" and dir_name in SKIPPED_ROOT_DIRS):
			continue
		# Like the editor, treat a folder with its own project.godot as another project.
		if not FileAccess.file_exists(path.path_join(".gdignore")) \
				and not FileAccess.file_exists(path.path_join("project.godot")):
			_collect_files(path, scripts, resources)
	for file_name: String in DirAccess.get_files_at(target):
		_collect_files(target.path_join(file_name), scripts, resources)


# The warnings set to "warn". Godot only reports warnings outside the editor when
# they are errors, and only reads these settings at startup, so the CLI promotes
# them through a temporary override.cfg and runs a second pass.
func _warning_settings() -> Array[String]:
	var settings: Array[String] = []
	for property: Dictionary in ProjectSettings.get_property_list():
		var name: String = property["name"]
		if name.begins_with(WARNINGS_PREFIX):
			var value: Variant = ProjectSettings.get_setting(name)
			if value is int and value == LEVEL_WARN:
				settings.append(name.trim_prefix("debug/"))
	return settings


func _load_scripts(scripts: Array[String]) -> Array[String]:
	var unloadable: Array[String] = []
	for path in scripts:
		var script := load(path) as GDScript
		if script == null or not script.can_instantiate():
			unloadable.append(path)
	return unloadable


func _report_script_errors(entries: Array[Dictionary], unloadable: Array[String], report_warnings: bool) -> void:
	var autoloads := _autoload_names()
	var explained := {}
	for entry: Dictionary in entries:
		var message: String = entry["message"]
		var file: String = entry["file"]
		explained[file] = true
		# The failure this refers to is reported on the script that caused it.
		if message.ends_with("Failed to compile depended scripts."):
			continue
		# Autoloads are not created yet, so the compile stage cannot find them.
		# The analyzer, which runs first, has already checked their use.
		if _is_missing_autoload_error(message, autoloads):
			continue
		var is_warning := message.ends_with(WARNING_SUFFIX)
		message = message.trim_prefix("Parse Error: ").trim_prefix("Compile Error: ").trim_suffix(WARNING_SUFFIX)
		if not report_warnings:
			# Warnings seen here are the ones the project itself set to "error".
			_add(file, entry["line"], "error", "gdscript", message)
		elif not is_warning:
			continue
		elif UNSAFE_MARKER in message:
			if _names_project_class(message):
				_add(file, entry["line"], "warning", "unsafe-access", message)
		else:
			_add(file, entry["line"], "warning", "gdscript-warning", message)
	if report_warnings:
		return
	for path in unloadable:
		if not explained.has(path):
			_add(path, 0, "error", "gdscript", "the script could not be loaded")


# Calling something Node or Variant does not declare is everyday dynamic GDScript,
# so it says little. On one of the project's own classes, it is usually a mistake.
func _names_project_class(message: String) -> bool:
	const PREFIX := "inferred type \""
	var start := message.find(PREFIX)
	if start == -1:
		return false
	var type := message.substr(start + PREFIX.length()).get_slice("\"", 0)
	return type != "Variant" and not ClassDB.class_exists(type)


func _is_missing_autoload_error(message: String, autoloads: Dictionary) -> bool:
	const PREFIX := "Compile Error: Identifier not found: "
	return message.begins_with(PREFIX) and autoloads.has(message.trim_prefix(PREFIX).strip_edges())


func _autoload_names() -> Dictionary:
	var names := {}
	for property: Dictionary in ProjectSettings.get_property_list():
		var name: String = property["name"]
		if name.begins_with("autoload/"):
			names[name.trim_prefix("autoload/")] = true
	return names


# A load() of a literal path only fails when the line runs, so the compiler says
# nothing about it.
func _check_string_references(scripts: Array[String]) -> void:
	var pattern := RegEx.create_from_string(
		"\\b(?:load|change_scene_to_file|ResourceLoader\\.load)\\(\\s*\"(res://[^\"]+)\""
	)
	for path in scripts:
		var lines := FileAccess.get_file_as_string(path).split("\n")
		for index in lines.size():
			var line := lines[index]
			var comment := line.find("#")
			for found: RegExMatch in pattern.search_all(line):
				if comment != -1 and comment < found.get_start():
					continue
				if not _resource_exists(found.get_string(1)):
					_add(path, index + 1, "error", "missing-resource", "\"%s\" does not exist" % found.get_string(1))


func _check_resources(resources: Array[String]) -> void:
	for path in resources:
		for dependency: String in ResourceLoader.get_dependencies(path):
			# Entries look like "uid://abc::Type::res://path" or just a path.
			var target: String = dependency.get_slice("::", 2) if "::" in dependency else dependency
			# A UID that still resolves wins over the path saved next to it, which
			# goes stale when a file moves.
			var uid: String = dependency.get_slice("::", 0) if "::" in dependency else ""
			if uid.begins_with("uid://") and _resource_exists(uid):
				continue
			if target != "" and not _resource_exists(target):
				_add(path, 0, "error", "missing-resource", "references \"%s\", which does not exist" % target)


func _check_project_settings() -> void:
	for property: Dictionary in ProjectSettings.get_property_list():
		var name: String = property["name"]
		if name.begins_with("autoload/"):
			var target: String = str(ProjectSettings.get_setting(name)).trim_prefix("*")
			if not _resource_exists(target):
				_add("res://project.godot", 0, "error", "missing-resource",
					"autoload %s points to \"%s\", which does not exist" % [name.trim_prefix("autoload/"), target])
	var main_scene: String = ProjectSettings.get_setting("application/run/main_scene", "")
	if main_scene != "" and not _resource_exists(main_scene):
		_add("res://project.godot", 0, "error", "missing-resource",
			"main scene \"%s\" does not exist" % main_scene)


func _resource_exists(path: String) -> bool:
	if path.begins_with("uid://"):
		return ResourceUID.has_id(ResourceUID.text_to_id(path))
	return FileAccess.file_exists(path) or ResourceLoader.exists(path)


func _add(file: String, line: int, severity: String, rule: String, message: String) -> void:
	# Godot often explains one mistake twice on the same line.
	var key := "%s:%d:%s" % [file, line, rule if rule != "missing-resource" else message]
	if not _seen.has(key):
		_seen[key] = true
		_diagnostics.append({
			"file": file, "line": line, "severity": severity, "rule": rule, "message": message,
		})

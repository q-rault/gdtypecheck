extends SceneTree

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
#
# A project's warning settings apply to this script too, so everything is typed
# explicitly: it has to compile under the strictest of them. The list of warnings
# differs between Godot versions, which rules out @warning_ignore.

const WARNINGS_PREFIX: String = "debug/gdscript/warnings/"
const WARNING_SUFFIX: String = " (Warning treated as error.)"
const LEVEL_WARN: int = 1
const LEVEL_ERROR: int = 2
const SKIPPED_ROOT_DIRS: Array[String] = ["addons"]
const RESOURCE_EXTENSIONS: Array[String] = ["tscn", "tres", "scn", "res"]
const UNSAFE_MARKER: String = "may be present on a subtype"
# Godot's warning messages, with the name of the warning each one comes from
# (debug/gdscript/warnings/<name>), in kebab-case. Godot reports a warning by its
# message only, so this is how a finding gets its code. Taken from Godot 4.7; a
# message from another version that matches none is reported without a code.
const WARNING_MESSAGES: Array[Array] = [
	["unassigned-variable", "The variable \"%s\" is used before being assigned a value."],
	["unassigned-variable-op-assign", "The variable \"%s\" is modified with the compound-assignment operator \"%s=\" but was not previously initialized."],
	["unused-variable", "The local variable \"%s\" is declared but never used in the block. If this is intended, prefix it with an underscore: \"_%s\"."],
	["unused-local-constant", "The local constant \"%s\" is declared but never used in the block. If this is intended, prefix it with an underscore: \"_%s\"."],
	["unused-private-class-variable", "The class variable \"%s\" is declared but never used in the class."],
	["unused-parameter", "The parameter \"%s\" is never used in the function \"%s()\". If this is intended, prefix it with an underscore: \"_%s\"."],
	["unused-signal", "The signal \"%s\" is declared but never explicitly used in the class."],
	["shadowed-variable", "The local %s \"%s\" is shadowing an already-declared %s at line %s in the current class."],
	["shadowed-variable-base-class", "The local %s \"%s\" is shadowing an already-declared %s at line %s in the base class \"%s\"."],
	["shadowed-variable-base-class", "The local %s \"%s\" is shadowing an already-declared %s in the base class \"%s\"."],
	["shadowed-global-identifier", "The %s \"%s\" has the same name as a %s."],
	["unreachable-code", "Unreachable code (statement after return) in function \"%s()\"."],
	["unreachable-pattern", "Unreachable pattern (pattern after wildcard or bind)."],
	["standalone-expression", "Standalone expression (the line may have no effect)."],
	["standalone-ternary", "Standalone ternary operator (the return value is being discarded)."],
	["incompatible-ternary", "Values of the ternary operator are not mutually compatible."],
	["untyped-declaration", "%s \"%s()\" has no static return type."],
	["untyped-declaration", "%s \"%s\" has no static type."],
	["inferred-declaration", "%s \"%s\" has an implicitly inferred static type."],
	["unsafe-property-access", "The property \"%s\" is not present on the inferred type \"%s\" (but may be present on a subtype)."],
	["unsafe-method-access", "The method \"%s()\" is not present on the inferred type \"%s\" (but may be present on a subtype)."],
	["unsafe-cast", "Casting \"Variant\" to \"%s\" is unsafe."],
	["unsafe-call-argument", "The argument %s of the %s \"%s()\" requires the subtype \"%s\" but the supertype \"%s\" was provided."],
	["unsafe-void-return", "The method \"%s()\" returns \"void\" but it's trying to return a call to \"%s()\" that can't be ensured to also be \"void\"."],
	["return-value-discarded", "The function \"%s()\" returns a value that will be discarded if not used."],
	["static-called-on-instance", "The function \"%s()\" is a static function but was called from an instance. Instead, it should be directly called from the type: \"%s.%s()\"."],
	["missing-tool", "The base class script has the \"@tool\" annotation, but this script does not have it."],
	["redundant-static-unload", "The \"@static_unload\" annotation is redundant because the file does not have a class with static variables."],
	["redundant-await", "\"await\" keyword is unnecessary because the expression isn't a coroutine nor a signal."],
	["missing-await", "\"await\" keyword might be desired because the expression is a coroutine."],
	["assert-always-true", "Assert statement is redundant because the expression is always true."],
	["assert-always-false", "Assert statement will raise an error because the expression is always false."],
	["integer-division", "Integer division. Decimal part will be discarded."],
	["narrowing-conversion", "Narrowing conversion (float is converted to int and loses precision)."],
	["int-as-enum-without-cast", "Integer used when an enum value is expected. If this is intended, cast the integer to the enum type using the \"as\" keyword."],
	["int-as-enum-without-match", "Cannot %s %s as Enum \"%s\": no enum member has matching value."],
	["enum-variable-without-default", "The variable \"%s\" has an enum type and does not set an explicit default value. The default will be set to \"0\"."],
	["empty-file", "Empty script file."],
	["deprecated-keyword", "The \"%s\" keyword is deprecated and will be removed in a future release. Please replace it with \"%s\"."],
	["confusable-identifier", "The identifier \"%s\" has misleading characters and might be confused with something else."],
	["confusable-local-declaration", "The %s \"%s\" is declared below in the parent block."],
	["confusable-local-usage", "The identifier \"%s\" will be shadowed below in the block."],
	["confusable-capture-reassignment", "Reassigning lambda capture does not modify the outer local variable \"%s\"."],
	["confusable-temporary-modification", "The built-in property \"%s\" will not be modified as a result of calling the method \"%s()\". Consider assigning the desired value to the property instead, or check the \"%s\" class for a specialized API."],
	["confusable-temporary-modification", "The built-in property \"%s\" will not be modified in the assignment chain. Consider using a temporary variable and assigning it back instead, or check the \"%s\" class for a specialized API."],
	["inference-on-variant", "The %s type is being inferred from a Variant value, so it will be typed as Variant."],
	["native-method-override", "The method \"%s()\" overrides a method from native class \"%s\". This won't be called by the engine and may not work as expected."],
	["get-node-default-without-onready", "The default value uses \"%s\" which won't return nodes in the scene tree before \"_ready()\" is called. Use the \"@onready\" annotation to solve this."],
	["onready-with-export", "\"@onready\" will set the default value after \"@export\" takes effect and will override it."],
]


class ErrorCollector extends Logger:
	var _mutex: Mutex = Mutex.new()
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
		if error_type != ERROR_TYPE_SCRIPT and error_type != ERROR_TYPE_SHADER:
			return
		_mutex.lock()
		entries.append({
			"file": file,
			"line": line,
			"message": rationale if rationale != "" else code,
			"is_shader": error_type == ERROR_TYPE_SHADER,
		})
		_mutex.unlock()


var _diagnostics: Array[Dictionary] = []
# [code, RegEx] for each of WARNING_MESSAGES, built when first needed.
var _warning_patterns: Array[Array] = []
var _seen: Dictionary = {}


func _init() -> void:
	var request_path: String = OS.get_cmdline_user_args()[0]
	var request: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(request_path))

	var scripts: Array[String] = []
	var resources: Array[String] = []
	var shaders: Array[String] = []
	for target: String in request["targets"]:
		_collect_files(target, scripts, resources, shaders)

	var report_warnings: bool = request["pass"] == "warnings"
	if report_warnings:
		var warnings: Array = request["warnings"]
		_turn_into_errors(warnings)

	var collector: ErrorCollector = ErrorCollector.new()
	OS.add_logger(collector)
	if not report_warnings:
		# Before the scripts: a shader they preload would compile only once.
		_check_shaders(shaders, collector)
	var unloadable: Array[String] = _load_scripts(scripts)
	OS.remove_logger(collector)
	_report_script_errors(collector.entries, unloadable, report_warnings)

	if not report_warnings:
		_check_string_references(scripts)
		_check_resources(resources)
		if request["whole_project"]:
			_check_project_settings()

	var output_path: String = request["output"]
	var output: FileAccess = FileAccess.open(output_path, FileAccess.WRITE)
	var _written: Variant = output.store_string(JSON.stringify({
		"files_checked": scripts.size(),
		"diagnostics": _diagnostics,
		"warning_settings": _warning_settings(),
	}))
	output.close()
	var _killed: Variant = OS.kill(OS.get_process_id())


func _collect_files(target: String, scripts: Array[String], resources: Array[String], shaders: Array[String]) -> void:
	if FileAccess.file_exists(target):
		if target.get_extension() == "gd":
			scripts.append(target)
		elif target.get_extension() in RESOURCE_EXTENSIONS:
			resources.append(target)
		elif target.get_extension() == "gdshader":
			shaders.append(target)
		return
	for dir_name: String in DirAccess.get_directories_at(target):
		var path: String = target.path_join(dir_name)
		if dir_name.begins_with(".") or (target == "res://" and dir_name in SKIPPED_ROOT_DIRS):
			continue
		# Like the editor, treat a folder with its own project.godot as another project.
		if not FileAccess.file_exists(path.path_join(".gdignore")) \
				and not FileAccess.file_exists(path.path_join("project.godot")):
			_collect_files(path, scripts, resources, shaders)
	for file_name: String in DirAccess.get_files_at(target):
		_collect_files(target.path_join(file_name), scripts, resources, shaders)


# Makes Godot report the warnings of `settings`, which it only does outside the
# editor when they are errors. This changes them for this process alone: nothing
# is saved, as other Godot processes, such as the game or its tests, may start
# meanwhile and must not see it. Scripts already compiled keep their warnings to
# themselves, hence a second pass.
func _turn_into_errors(settings: Array) -> void:
	for setting: String in settings:
		ProjectSettings.set_setting(setting, LEVEL_ERROR)
	# Godot 4.6 and newer read the levels at startup and on this signal, which
	# would come too late: at the end of the frame.
	ProjectSettings.settings_changed.emit()


# The warnings set to "warn", which the second pass turns into errors.
func _warning_settings() -> Array[String]:
	var settings: Array[String] = []
	for property: Dictionary in ProjectSettings.get_property_list():
		var name: String = property["name"]
		if name.begins_with(WARNINGS_PREFIX):
			var value: Variant = ProjectSettings.get_setting(name)
			if value is int and value == LEVEL_WARN:
				settings.append(name)
	return settings


# Shaders compile on first use, even with the dummy renderer of a headless run.
# Godot reports errors in the shader itself without a file, and errors in an
# included file with that file's path.
func _check_shaders(shaders: Array[String], collector: ErrorCollector) -> void:
	for path: String in shaders:
		if _has_missing_include(path):
			continue
		var shader: Shader = load(path) as Shader
		if shader == null:
			_add(path, 0, "error", "shader", "the shader could not be loaded", "shader-error")
			continue
		var _uniforms: Variant = shader.get_shader_uniform_list()
		for entry: Dictionary in collector.entries:
			if entry["is_shader"]:
				var file: String = entry["file"]
				var line: int = entry["line"]
				var message: String = entry["message"]
				_add(file if file != "" else path, line, "error", "shader", message, "shader-error")
		collector.entries.clear()


# Godot reports a missing include as an unknown character at the "#", so it is
# looked for first to report the actual cause.
func _has_missing_include(path: String) -> bool:
	var pattern: RegEx = RegEx.create_from_string("^\\s*#include\\s+\"([^\"]+)\"")
	var lines: PackedStringArray = FileAccess.get_file_as_string(path).split("\n")
	var is_missing: bool = false
	for index: int in lines.size():
		var found: RegExMatch = pattern.search(lines[index])
		if found != null:
			var include: String = found.get_string(1)
			var resolved: String = include if include.begins_with("res://") else path.get_base_dir().path_join(include)
			if not _resource_exists(resolved):
				_add(path, index + 1, "error", "missing-resource", "\"%s\" does not exist" % include)
				is_missing = true
	return is_missing


func _load_scripts(scripts: Array[String]) -> Array[String]:
	var unloadable: Array[String] = []
	for path: String in scripts:
		var script: GDScript = load(path) as GDScript
		if script == null or not script.can_instantiate():
			unloadable.append(path)
	return unloadable


func _report_script_errors(entries: Array[Dictionary], unloadable: Array[String], report_warnings: bool) -> void:
	var autoloads: Dictionary = _autoload_names()
	var explained: Dictionary = {}
	for entry: Dictionary in entries:
		if entry["is_shader"]:
			continue
		var message: String = entry["message"]
		var file: String = entry["file"]
		var line: int = entry["line"]
		explained[file] = true
		# The failure this refers to is reported on the script that caused it.
		if message.ends_with("Failed to compile depended scripts."):
			continue
		# Autoloads are not created yet, so the compile stage cannot find them.
		# The analyzer, which runs first, has already checked their use.
		if _is_missing_autoload_error(message, autoloads):
			continue
		var is_warning: bool = message.ends_with(WARNING_SUFFIX)
		message = message.trim_prefix("Parse Error: ").trim_prefix("Compile Error: ").trim_suffix(WARNING_SUFFIX)
		var code: String = _warning_code(message) if is_warning else ""
		if not report_warnings:
			# Warnings seen here are the ones the project itself set to "error".
			# Godot gives its errors no names, and calls type errors "Parse Error"
			# too, so they all get one code.
			if not is_warning:
				code = "gdscript-error"
			_add(file, line, "error", "gdscript", message, code)
		elif not is_warning:
			continue
		elif UNSAFE_MARKER in message:
			if _names_project_class(message):
				_add(file, line, "warning", "unsafe-access", message)
		else:
			_add(file, line, "warning", "gdscript-warning", message, code)
	if report_warnings:
		return
	for path: String in unloadable:
		if not explained.has(path):
			_add(path, 0, "error", "gdscript", "the script could not be loaded", "load-error")


# Calling something Node or Variant does not declare is everyday dynamic GDScript,
# so it says little. On one of the project's own classes, it is usually a mistake.
func _names_project_class(message: String) -> bool:
	const PREFIX: String = "inferred type \""
	var start: int = message.find(PREFIX)
	if start == -1:
		return false
	var type: String = message.substr(start + PREFIX.length()).get_slice("\"", 0)
	return type != "Variant" and not ClassDB.class_exists(type)


func _is_missing_autoload_error(message: String, autoloads: Dictionary) -> bool:
	const PREFIX: String = "Compile Error: Identifier not found: "
	return message.begins_with(PREFIX) and autoloads.has(message.trim_prefix(PREFIX).strip_edges())


func _autoload_names() -> Dictionary:
	var names: Dictionary = {}
	for property: Dictionary in ProjectSettings.get_property_list():
		var name: String = property["name"]
		if name.begins_with("autoload/"):
			names[name.trim_prefix("autoload/")] = true
	return names


# A load() of a literal path only fails when the line runs, so the compiler says
# nothing about it.
func _check_string_references(scripts: Array[String]) -> void:
	var pattern: RegEx = RegEx.create_from_string(
		"\\b(?:load|change_scene_to_file|ResourceLoader\\.load)\\(\\s*\"(res://[^\"]+)\""
	)
	for path: String in scripts:
		var lines: PackedStringArray = FileAccess.get_file_as_string(path).split("\n")
		for index: int in lines.size():
			var line: String = lines[index]
			var comment: int = line.find("#")
			for found: RegExMatch in pattern.search_all(line):
				if comment != -1 and comment < found.get_start():
					continue
				if not _resource_exists(found.get_string(1)):
					_add(path, index + 1, "error", "missing-resource", "\"%s\" does not exist" % found.get_string(1))


func _check_resources(resources: Array[String]) -> void:
	for path: String in resources:
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


func _add(file: String, line: int, severity: String, rule: String, message: String, code: String = "") -> void:
	# Godot often explains one mistake twice on the same line.
	var key: String = "%s:%d:%s" % [file, line, rule if rule != "missing-resource" else message]
	if not _seen.has(key):
		_seen[key] = true
		var diagnostic: Dictionary = {
			"file": file, "line": line, "severity": severity, "rule": rule, "message": message,
		}
		if code != "":
			diagnostic["code"] = code
		_diagnostics.append(diagnostic)


# The name of the Godot warning `message` comes from, or "" when it is unknown.
func _warning_code(message: String) -> String:
	if _warning_patterns.is_empty():
		for pair: Array in WARNING_MESSAGES:
			var code: String = pair[0]
			var template: String = pair[1]
			_warning_patterns.append([code, RegEx.create_from_string(_pattern(template))])
	for pair: Array in _warning_patterns:
		var code: String = pair[0]
		var regex: RegEx = pair[1]
		if regex.search(message) != null:
			return code
	return ""


# A regular expression matching a message made from `template`, whose "%s" can
# be anything.
func _pattern(template: String) -> String:
	var escaped: String = ""
	for index: int in template.length():
		var character: String = template[index]
		escaped += "\\" + character if character in "\\^$.|?*+()[]{}" else character
	return "^" + escaped.replace("%s", ".*?") + "$"

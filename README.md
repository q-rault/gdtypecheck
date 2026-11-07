# gdtypecheck

Reports what is wrong with a Godot project without opening the editor or running the game: GDScript compile errors, including type errors, and references to files that no longer exist. It is meant for CI and coding agents, which can't see the editor's error underlines.

```console
$ gdtypecheck
player/player.gd:42: error[gdscript]: Cannot return a value of type "String" as "int".
ui/hud.tscn: error[missing-resource]: references "res://ui/old_bar.gd", which does not exist
levels/menu.gd:8: error[missing-resource]: "res://levels/gone.tscn" does not exist
enemy.gd:17: warning[gdscript-warning]: The local variable "speed" is declared but never used in the block.
gdtypecheck: 3 error(s), 1 warning(s) (158 scripts)
```

## Install

```sh
cargo install gdtypecheck
```

It needs Godot 4.5 or later, found through `--godot-exe`, `$GODOT_EXE`, or `godot` on the `PATH`.

## Usage

```sh
gdtypecheck                     # the whole project
gdtypecheck player/ ui/hud.gd   # only these files and folders
gdtypecheck --strict            # also check members of your own classes
gdtypecheck --format json       # for tools and agents
```

| Exit code | Meaning |
| --- | --- |
| `0` | No problems (warnings don't count unless you pass `--warnings-as-errors`) |
| `1` | Problems found |
| `2` | The check could not run |

## What it reports

| Rule | What it finds | When |
| --- | --- | --- |
| `gdscript` | Parse and type errors, wrong arguments, unknown names and types, missing `preload`s | Always |
| `missing-resource` | Scenes and resources that reference missing files, broken autoloads and main scene, `load("res://...")` and `change_scene_to_file("res://...")` of missing files | Always |
| `gdscript-warning` | The warnings your project settings set to "warn" | Unless `--no-warnings` |
| `unsafe-access` | Methods and properties that don't exist on one of your own classes | With `--strict` |

## How it works, and what it can't see

It loads your scripts in a headless Godot and collects what Godot's own compiler reports, so the type analysis is Godot's and matches the editor. That has limits:

- **Only typed code is checked.** `var p = Player.new()` is unchecked; `var p := Player.new()` is. A typed project gets more out of this tool.
- **`--strict` covers your classes only.** A method missing from `Node` or `Variant` is usually normal dynamic GDScript, so those aren't reported. A property that only a subclass declares is reported, even though it can work at runtime.
- **Literal paths only.** A `load()` of a path built at runtime isn't checked.
- **Node paths aren't checked.** `$Missing` and `get_node("Missing")` are not compared with your scenes.
- **It finds errors, not bugs.** Whether the code does the right thing is what tests are for.

## Effects on your project

- Your code doesn't run: the check ends before Godot creates autoloads or scenes. The exception is **static variable initializers**, which run when a script loads.
- When the class cache in `.godot/` is missing or out of date, it runs `godot --import`, as the editor would. This can create `.uid` files for new scripts.
- Reporting warnings takes a second Godot run with a temporary `override.cfg` in the project root, which is the only way to change warning levels. It's restored afterwards, and if a run is killed, the next one cleans it up. `--no-warnings` skips it, which is about twice as fast, so avoid running two checks of one project at once without it.

Addons, hidden folders, folders with a `.gdignore`, and nested projects are skipped unless you name them as targets or your code depends on them.

## Part of gdsuite

`gdtypecheck` works alone, and is also the typecheck step of [`gdsuite`](https://github.com/q-rault/gdsuite).

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

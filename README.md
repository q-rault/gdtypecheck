# gdtypecheck

`gdtypecheck` is a standalone CLI that performs a Godot-driven GDScript load/type check.

Current behavior:

- accepts file and directory targets
- discovers `.gd` files under directory targets
- skips common generated/vendor folders such as `addons/`, `.godot/`, `reports/`, and `.gdsuite/`
- generates its own checker script under `.gdtypecheck/state/`
- invokes Godot in headless mode and fails if any script cannot be loaded

Typical usage:

```bash
gdtypecheck .
gdtypecheck test/ client/
gdtypecheck --godot-exe /path/to/godot .
```

For local `gdsuite` development, point `gdsuite.toml` at this repo with:

```toml
[tools]
gdtypecheck_source = "path:../gdtypecheck"
```

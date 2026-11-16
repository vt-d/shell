# Theming with matugen

The shell reads its colors from `~/.config/shell/theme.json` (respecting
`$XDG_CONFIG_HOME`), which holds matugen output. Without that file it uses a built-in
dark theme.

## One-off

```sh
mkdir -p ~/.config/shell
matugen --source-color-index 0 image ~/wallpaper.png --json hex > ~/.config/shell/theme.json
```

## On every wallpaper change

Copy `shell.json` from this directory to `~/.config/matugen/templates/` and add to
`~/.config/matugen/config.toml`:

```toml
[templates.shell]
input_path = '~/.config/matugen/templates/shell.json'
output_path = '~/.config/shell/theme.json'
```

The shell watches that file and recolors itself as soon as matugen rewrites it.

For Nix/Home Manager integration, see [the project README](../README.md).
Manage the template and Matugen config declaratively, but leave the generated
`shell/theme.json` writable so wallpaper changes can update it without rebuilding.

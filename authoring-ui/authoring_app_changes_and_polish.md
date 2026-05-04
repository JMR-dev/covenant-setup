# Changes and Polish

- Expected convention for toml file names is ${AppName}-install.toml. Rust installer engine will enforce this.
- There is way too much whitespace on the left column by default. Padding should be minimal and sections should expand as items are added
- File paths and objects should be removable through a x button to the right of the file path with outbossed borders around each file path/entity added
- Folder text box should be renamed "Application Target Installation Folder"
- Application should support both dark and light mode, with a toggle button at the top to switch between those themes. It should follow the user's OS light/dark theme by default, but as soon as the user changes the toggle, it saves a preferences.ini file that remembers which way the user toggled the theme. This file should update on every press of the toggle button.

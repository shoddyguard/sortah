# sortah

> This project was mainly made as a way for me to dick around in Rust before doing something more serious and is not at all a serious tool.

Sort images downloaded from social media into per-person folders. sortah extracts the username from each filename, matches it against a configurable alias mapping, and moves the file into the right directory.

One person can have many usernames (`joeBloggs`, `joe_bloggs`, `jblgs`); all their images end up in the same folder.

## Features

- Map multiple usernames to a single person; they all sort into the same folder
- Shows a plan before moving anything, then prompts for confirmation (or `--yes` to skip)
- Identical duplicates are skipped; name clashes are automatically renamed
- Bulk import/export via CSV

## Installation

Download the latest binary from the [Releases](../../releases) page.

## Quick start

```sh
# 1. Write a starter config and create an empty database
sortah config init

# 2. Edit the config
$EDITOR ~/.config/sortah/config.yaml

# 3. Load your existing mapping from a CSV file
sortah import people.csv

# 4. Check everything looks right
sortah list
sortah config validate

# 5. cd to your downloads folder and sort
cd ~/Downloads/friend-pics
sortah sort          # shows a plan, then prompts before moving
sortah sort --yes    # skip the prompt
```

## Commands

```
sortah sort                    Sort the current directory
sortah sort --yes              Sort without the confirmation prompt
sortah sort --dest <path>      Override destination_root for this run
sortah sort --verbose          Print every planned move

sortah config init             Write starter config and create database
sortah config path             Print config and database paths
sortah config validate         Validate config and report any issues

sortah person add <name>       Add a person
sortah person rm <name>        Remove a person (and their aliases)

sortah alias add <name> <alias>  Map an alias to a person
sortah alias rm <alias>          Remove an alias

sortah list                    List all people and aliases
sortah list --person <name>    List aliases for one person

sortah import <file.csv>       Bulk-import from CSV
sortah export <file.csv>       Export mapping to CSV
```

Global flags (work with every command):

```
-c, --config <path>   Use this config file instead of the default
    SORTAH_CONFIG      Environment variable equivalent of --config
```

## Config file

Default path: `~/.config/sortah/config.yaml` (Linux), `~/Library/Application Support/sortah/config.yaml` (macOS), `%APPDATA%\sortah\config.yaml` (Windows).

Override with `--config <path>` or the `SORTAH_CONFIG` environment variable.

```yaml
# Where sorted images will be placed. Each person gets a subdirectory here.
destination_root: ~/Pictures/Friends

# Whether to match aliases case-insensitively against filenames.
# When true, alias "joeBloggs" matches a file containing "joebloggs".
case_insensitive: true

# Image extensions to process (case-insensitive).
extensions: [jpg, jpeg, png, gif, webp, mp4]

# Path to the alias database. Defaults to the platform data directory when omitted.
# database: ~/.local/share/sortah/mappings.db
```

## Managing people and aliases

```sh
# Add a person
sortah person add "Joe Bloggs"

# Add their username aliases (stored exactly as typed)
sortah alias add "Joe Bloggs" joeBloggs
sortah alias add "Joe Bloggs" joe_bloggs
sortah alias add "Joe Bloggs" jblgs

# Remove an alias
sortah alias rm jblgs

# Remove a person (also removes all their aliases)
sortah person rm "Joe Bloggs"

# List everything
sortah list
sortah list --person "Joe Bloggs"
```

## CSV bulk import / export

One row per person: the name followed by all their aliases as additional columns.

```csv
name,aliases
Joe Bloggs,joeBloggs,joe_bloggs,jblgs
Jane Doe,janedoe,jane.d
```

```sh
sortah import people.csv     # create people and aliases in bulk
sortah export backup.csv     # dump the full mapping for backup or editing
```

Import is idempotent: exact-duplicate aliases are skipped.

## Sort behaviour

`sortah sort` scans the current working directory recursively for image files and builds a plan:

| File situation | Action |
|---|---|
| Username matches an alias | Planned for move to `destination_root/<name>/` |
| Username not in mapping | Left in place, reported |
| Filename does not match the regex | Left in place, reported |
| Destination file is identical | Skipped (duplicate) |
| Destination file differs (name clash) | Renamed `file (2).jpg`, `file (3).jpg`, etc. |

The plan is printed with a per-person breakdown before anything is moved. Confirm with `y` or pass `--yes` to skip the prompt. Use `--verbose` to see every planned move.

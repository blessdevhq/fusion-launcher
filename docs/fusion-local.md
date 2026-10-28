# Local Fusion Source development

`fusion-local` runs packer output through the same FusionService, manifest checks,
CAS, workspace, persistence, integrity guard and supervisor as the desktop engine.
It reads an explicitly selected local source directory without a network fallback.
The desktop app can now connect these directories too, using its normal library
and game details actions. No game-specific profile is required.

## Desktop workflow

1. Open Settings → Sources → **Choose source folder** and select the packer output
   directory containing `source-index.json` and `artifacts/`. Review its index and
   click **Add source**. This does not install any game.
2. Open the game in Library. Select optional layers and any required user files,
   then install. Review and approve the exact executable hashes before launching.
3. To publish another local configuration, build a complete new packer output with
   the same source/game IDs and publisher key. In the connected source card, choose
   **Choose another release folder**. This changes the available catalog revision;
   installed games remain on their current snapshots until **Update** is selected.
4. Use **Apply options** to rebuild with another set of optional mods, **Launch** to
   start the supervised process, and **Roll back binaries** to restore the previous
   snapshot while retaining current saves.

Directory bindings are host data stored in SQLite schema v5. They survive launcher
restarts. A source cannot replace another source's ID/key, grant executable trust,
or assign itself an arbitrary local path. Replacing a directory is refused while
an operation is active. The reserved `offline.fusion.invalid` identity uses only
the connected directory and never falls back to HTTP when it is missing.

Local packer recipes use `base_url: https://offline.fusion.invalid/<source-id>/`.
The directory is not copied into the launcher; keep it available for future
installations/updates. The existing CLI's private database/installations are
separate and are not automatically adopted into the desktop library.

For a source containing portable Eden profiles, explicitly enable **Prepare
portable Eden profiles** when connecting it. This host option relocates storage
paths and registers the workspace game/update directory inside the isolated
profile, under the same workspace lease as launch. It requires `runtime/eden.exe`
and persistent `workspace/runtime/user`; ordinary sources leave it disabled.
Sources cannot enable this hook through manifest fields. For the existing private
TOTK example, select `.tmp/totk-fusion/ready/source-v2`; choose a short workspace
on F: in the game's existing workspace setting to stay within Eden's path budget.
This does not import or modify original global Eden profiles/saves.

Build:

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --bin fusion-packer --bin fusion-local
```

Build the first source release using a normal packer recipe (see
[authoring](fusion-packer.md)). For local transport, set its `base_url` to
`https://offline.fusion.invalid/<source_info.id>/` and use artifacts under that
base. The URL is an internal identity, not a server to contact.

```powershell
./src-tauri/target/debug/fusion-packer.exe build --recipe ./recipe.json --out ./dist-v1
./src-tauri/target/debug/fusion-local.exe install --source-dir ./dist-v1 --library ./private-library --game demo
./src-tauri/target/debug/fusion-local.exe status --source-dir ./dist-v1 --library ./private-library --game demo
```

Executable approval remains explicit: use `trust --path ... --blake3 ...` after
reviewing the exact file digest. Publishing a manifest does not approve binaries.
`launch` waits for the process tree and reconciles saves on exit. `launch-eden`
additionally prepares the existing portable Eden profile layout; it is optional.

## Change selected mods

Optional mod layers already belong to the Source schema. Create a separate
`selection.json` with the existing camelCase GameOptions contract:

```json
{"enabledAddons":["graphics"],"runtimeChannel":null}
```

`enabledAddons: []` disables optional additions; `null` uses manifest defaults.
Required layers remain selected. Staging validates dependencies/conflicts without
replacing the installed snapshot:

```powershell
./src-tauri/target/debug/fusion-local.exe options --options-file ./selection.json --source-dir ./dist-v1 --library ./private-library --game demo
./src-tauri/target/debug/fusion-local.exe update --source-dir ./dist-v1 --library ./private-library --game demo
```

`status` distinguishes `options` from `appliedOptions`. `update` applies the staged
selection, including when the package version is unchanged. It retains the
installed workspace location and uses the normal transactional installation path.

## Release another configuration

Change the recipe's game version and the relevant config templates/mod artifacts,
then build into a new output directory. Keep the source ID, game ID and publisher
identity stable. The new output must be complete, including unchanged artifacts.

```powershell
./src-tauri/target/debug/fusion-packer.exe build --recipe ./recipe-v2.json --out ./dist-v2
./src-tauri/target/debug/fusion-local.exe status --source-dir ./dist-v2 --library ./private-library --game demo
./src-tauri/target/debug/fusion-local.exe update --source-dir ./dist-v2 --library ./private-library --game demo
./src-tauri/target/debug/fusion-local.exe rollback --source-dir ./dist-v2 --library ./private-library --game demo
```

Opening the host validates/registers the selected source index. `status` can then
report an available revision; it does not install it. Update creates a new active
snapshot; rollback restores the previous snapshot. Both retain current data in
declared persistence paths. A changed executable requires approval for its new
digest. Reverting to an already approved digest restores that digest's trust.

Keep package-owned generated configuration outside mutable persistence paths.
Persistent user settings remain user data: replacing a template does not promise
to merge or overwrite an emulator's persistent INI. Such migrations need an
explicit policy. This host also does not add a CLI importer for `user_provided`
layers; the desktop service's existing InstallRequest handles those inputs.

## Development acceptance

Accept the Source lifecycle independently of any particular game's full QA:
install, change optional layers, update package configuration, preserve current
saves, enforce digest trust, and roll back. A game's graphics quality, late-game
mechanics and publication readiness are separate checks. A provisional personal
TOTK recipe can exercise integration without becoming a polished public preset.

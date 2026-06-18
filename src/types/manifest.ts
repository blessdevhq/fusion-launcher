// Mirrors the Rust `Manifest` returned by the `fetch_manifest` command.
// The backend serializes these structs without `rename_all`, so the JSON
// keys are snake_case — keep them snake_case here to match the payload.

export interface ManifestVisuals {
  cover_url?: string;
  background_url?: string;
}

export interface ManifestAssets {
  heavy_rom_magnet?: string | null;
  core_bundle_p2p_hash?: string | null;
  shader_cache_url?: string | null;
  core_bundle_url?: string | null;
  core_bundle_sha256?: string | null;
}

export interface ManifestLaunchConfig {
  engine?: string;
  executable?: string;
  args?: string[];
  inject_mods?: string[];
}

export interface ManifestGame {
  title_id: string;
  title: string;
  platform: string;
  game_version?: string;
  visuals?: ManifestVisuals;
  assets: ManifestAssets;
  launch_config?: ManifestLaunchConfig;
}

export interface Manifest {
  manifest_version: string;
  repository_name: string;
  last_updated: string;
  games: ManifestGame[];
}

// Shape of the structured error the command rejects with on failure.
export interface ManifestError {
  kind: 'invalid_url' | 'network' | 'parse';
  message: string;
}

import { z } from 'zod';

/** Structural authoring contracts only; parsing never authorizes execution. */
const text = (min: number, max: number) => z.string().refine((value) => {
  const length = [...value].length;
  return length >= min && length <= max;
}, 'String length is outside the frozen contract');
const id = text(1, 128);
const hash = z.string().regex(/^[a-f0-9]{64}$(?![\s\S])/);
const clean = text(0, 2048).refine((value) => !/[\u0000-\u001f\u007f-\u009f]/.test(value));
const path = text(1, 512).refine((value) => /^(?!\/)(?!.*[\\:\u0000-\u001f\u007f-\u009f<>"|?*])(?!(?:[\s\S]*\/)?\.{1,2}(?:\/|$))(?!(?:[\s\S]*\/)?[^/]*[. ](?:\/|$))(?!(?:[\s\S]*\/)?(?:[cC][oO][nN]|[pP][rR][nN]|[aA][uU][xX]|[nN][uU][lL]|[cC][oO][mM][1-9]|[lL][pP][tT][1-9])(?:\.[^/]*|)(?:\/|$))[^/]+(?:\/[^/]+)*(?![\s\S])/.test(value));
const directory = z.union([z.literal(''), path]);
const list = <T extends z.ZodTypeAny>(schema: T, min: number, max: number, unique = false) =>
  z.array(schema).min(min).max(max).superRefine((items, ctx) => {
    if (unique && new Set(items.map((item) => JSON.stringify(item))).size !== items.length) {
      ctx.addIssue({ code: z.ZodIssueCode.custom, message: 'Duplicate array items' });
    }
  });
const scalar = z.union([z.boolean(), z.number().finite().min(-Number.MAX_SAFE_INTEGER).max(Number.MAX_SAFE_INTEGER), clean]);
// Assert URI syntax before WHATWG parsing can normalize an IRI or malformed escape.
const httpsUri = new RegExp(String.raw`^[hH][tT][tT][pP][sS]://(?:[A-Za-z0-9._~!$&'()*+,;=:\[\]-]|%[A-Fa-f0-9]{2})+(?:/(?:[A-Za-z0-9._~!$&'()*+,;=:@/-]|%[A-Fa-f0-9]{2})*)?(?:\?(?:[A-Za-z0-9._~!$&'()*+,;=:@/?-]|%[A-Fa-f0-9]{2})*)?(?:#(?:[A-Za-z0-9._~!$&'()*+,;=:@/?-]|%[A-Fa-f0-9]{2})*)?$(?![\s\S])`);
const https = z.string().refine((value) => {
  if (!httpsUri.test(value)) return false;
  try {
    const url = new URL(value);
    return url.protocol === 'https:' && !!url.hostname && !url.username && !url.password;
  } catch {
    return false;
  }
});
const origin = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('managed'), artifact_ref: id, redistribution_allowed: z.boolean() }).strict(),
  z.object({ kind: z.literal('external'), url: https, transport_blake3: hash, inventory_blake3: hash }).strict(),
  z.object({ kind: z.literal('user_provided'), requirement_id: id, inventory_blake3: hash, instructions: z.string() }).strict()
]);
const binding = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('keyboard'), code: id }).strict(),
  z.object({ kind: z.literal('mouse_button'), button: z.number().int().nonnegative() }).strict(),
  z.object({ kind: z.literal('mouse_axis'), axis: z.enum(['x', 'y']), direction: z.enum(['positive', 'negative']) }).strict(),
  z.object({ kind: z.literal('controller_button'), button: id }).strict(),
  z.object({ kind: z.literal('controller_axis'), axis: id, direction: z.enum(['positive', 'negative']) }).strict()
]);
// Zod 3 records drop __proto__. Preserve own JSON keys without assigning to a prototype setter.
export function preservingJsonRecord<T extends z.ZodTypeAny>(schema: T) {
  return z.custom<Record<string, unknown>>((value) => {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) return false;
    const prototype = Object.getPrototypeOf(value);
    return prototype === Object.prototype || prototype === null;
  }).transform((value) => Object.entries(value))
    .pipe(z.array(z.tuple([z.string(), schema])))
    .transform((entries) => Object.fromEntries(entries));
}
const bindings = preservingJsonRecord(binding);
const xmlName = text(1, 128).refine((value) => /^[A-Za-z_][A-Za-z0-9_.-]*(?![\s\S])/.test(value));
const iniSection = text(1, 128).refine((value) => /^[A-Za-z0-9_. /-]+(?![\s\S])/.test(value));
const iniKey = text(1, 128).refine((value) => /^[A-Za-z0-9_. \\/-]+(?![\s\S])/.test(value));
const address = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('ini'), section: iniSection, key: iniKey }).strict(),
  z.object({ kind: z.literal('xml_attribute'), elements: list(xmlName, 0, 8), attribute: xmlName }).strict()
]);
const seed = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('empty_ini') }).strict(),
  z.object({ kind: z.literal('xml_root'), root: xmlName,
    attributes: list(z.object({ name: xmlName, value: scalar }).strict(), 0, 16) }).strict()
]);
const config = z.object({
  id, path, format: z.enum(['ini', 'xml_attributes']), seed, preserve_unmanaged: z.literal(true),
  keys: list(z.object({ id, address, scalar_type: z.enum(['boolean', 'integer', 'number', 'string']),
    allowed_values: list(scalar, 1, 64, true) }).strict(), 0, 128)
}).strict();
const controls = z.discriminatedUnion('kind', [
  z.object({ id, title: text(0, 256), kind: z.literal('application_defaults') }).strict(),
  z.object({ id, title: text(0, 256), kind: z.literal('eden_param_package_v1'),
    device: z.enum(['keyboard_mouse', 'sdl_gamecontroller']), bindings, config_id: id }).strict()
]);
const protectedEnvironment = new Set(['PATH', 'PATHEXT', 'COMSPEC', 'SYSTEMROOT', 'WINDIR', 'APPDATA',
  'LOCALAPPDATA', 'USERPROFILE', 'HOME', 'TEMP', 'TMP', 'QT_PLUGIN_PATH', 'QT_QPA_PLATFORM_PLUGIN_PATH',
  'PYTHONPATH', 'PYTHONHOME', 'NODE_OPTIONS', 'RUSTC_WRAPPER']);

export const programDescriptorSchema = z.object({
  document_type: z.literal('program_descriptor'), schema_version: z.literal('1.0'),
  identity: z.object({ id, title: text(1, 256), version: id, os: z.literal('windows'),
    architecture: z.enum(['x86_64', 'x86', 'aarch64', 'armv7']) }).strict(),
  executor: z.object({ id: z.literal('fusion.declarative'), version: z.literal('1.0') }).strict(),
  components: list(z.object({ id, layer_id: id, origin, source_root: directory, mount_path: directory }).strict(), 1, 64),
  required_files: list(z.object({ path, role: z.enum(['data', 'main_executable', 'core_library']) }).strict(), 1, 256),
  projection_rules: list(z.object({ path, winning_layer: id }).strict(), 0, 4096),
  directories: list(path, 0, 64, true),
  entrypoint: z.object({
    executable: path, working_directory: directory,
    arguments: list(z.discriminatedUnion('kind', [
      z.object({ kind: z.literal('literal'), value: clean }).strict(),
      z.object({ kind: z.literal('workspace_path'), path: directory,
        prefix: text(0, 64).refine((value) => /^(?:|--?[A-Za-z0-9_-]+=)(?![\s\S])/.test(value)) }).strict()
    ]), 0, 64),
    environment_paths: list(z.object({
      name: text(1, 64).refine((value) => /^[A-Z][A-Z0-9_]*(?![\s\S])/.test(value) && !protectedEnvironment.has(value)),
      path: directory
    }).strict(), 0, 32)
  }).strict(),
  configs: list(config, 0, 16),
  build_profiles: list(z.object({ id, title: text(0, 256), layer_ids: list(id, 1, 64, true),
    controls_profile_ids: list(id, 1, 16, true), settings: list(z.object({ config_id: id, key_id: id, value: scalar }).strict(), 0, 256)
  }).strict(), 1, 32),
  controls_profiles: list(controls, 1, 16),
  persistence: list(z.object({ path, kind: z.enum(['file', 'directory']), policy: z.literal('preserve_latest'),
    export: z.literal('never') }).strict(), 0, 64)
}).strict();

export const creatorProgramProjectSchema = z.object({
  schema_version: z.literal('1.1'), project_id: id,
  revision: z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER), title: z.string(),
  source_id: id, game_id: id, game_version: id, program_descriptor: programDescriptorSchema
}).strict();
export const programCreatorManifestSchema = z.object({ program_descriptor: programDescriptorSchema }).strict();
export type ProgramDescriptor = z.infer<typeof programDescriptorSchema>;
export type CreatorProgramProject = z.infer<typeof creatorProgramProjectSchema>;
export type ProgramCreatorManifest = z.infer<typeof programCreatorManifestSchema>;

// Serialized shapes match the Rust catalog and develop commands; Rust retains validation.
export type Point = [number, number];
export type Crop = [number, number, number, number];
export type Focus = { x: number; y: number };
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export type Module = 'library' | 'develop';
export type ViewMode = 'grid' | 'loupe' | 'compare' | 'survey';
export interface Photo { id: number; root_id: number; path: string; name: string; extension: string; size: number; modified: number; added: number; missing: boolean; rating: number; flag: number; label: string }
export interface Root { id: number; path: string }
export interface Folder { root_id: number; path: string; count: number }
export interface Query { ids: number[]; text: string; root_id: number | null; folder: string | null; rating: number | null; rating_mode: string; flags: number[]; labels: string[]; metadata: Record<string, string[]>; extensions?: string[]; tags?: string[]; added_from?: number | null; added_to?: number | null; include_missing?: boolean; sort: string; descending: boolean }
export interface Cursor { generation: number; query: Query; key: Json; id: number }
export interface Page { items: Photo[]; next: Cursor | null; generation: number; adjusted_ids: number[] }
export interface Detail { item: Photo; tags: string[]; memo: string }
export interface Preview { path: string; width: number; height: number; render_width: number; render_height: number; inline_image?: string; pending?: boolean; unavailable?: boolean; error?: string; bound?: number }
export interface PhotoMetadata { [key: string]: string }
export interface Shape { kind: string; operation: string; start: Point; end: Point; radius: number; feather: number; opacity: number; points: Point[]; color: [number, number, number]; range: Point }
export interface Mask { name: string; enabled: boolean; invert: boolean; amount: number; shapes: Shape[]; adjustments: Record<string, number> }
export interface Settings {
  version: number;
  masks: Mask[];
  aspect: number;
  offset_x: number;
  offset_y: number;
  constrain_crop: boolean;
  exposure: number;
  temperature: number;
  tint: number;
  raw_temperature: number | null;
  raw_tint: number;
  raster_temperature: number | null;
  raster_reference_temperature: number | null;
  raster_tint: number;
  contrast: number;
  highlights: number;
  shadows: number;
  whites: number;
  blacks: number;
  texture: number;
  clarity: number;
  dehaze: number;
  vibrance: number;
  saturation: number;
  monochrome: boolean;
  curves: number[][];
  point_curves: Point[][];
  mixer: number[][];
  grading: number[][];
  grading_balance: number;
  grading_blending: number;
  sharpen: number;
  sharpen_radius: number;
  sharpen_detail: number;
  sharpen_masking: number;
  noise_luma: number;
  noise_color: number;
  crop: Crop;
  quarter_turns: number;
  flip_h: boolean;
  flip_v: boolean;
  angle: number;
  perspective_h: number;
  perspective_v: number;
  scale: number;
  distortion: number;
  vignette: number;
  vignette_midpoint: number;
  vignette_feather: number;
  grain: number;
  grain_size: number;
  grain_roughness: number;
}
export type SettingsPatch = Partial<Settings>;
export type SettingKey = keyof Settings;
export type NumericSetting = { [K in SettingKey]: Settings[K] extends number | null ? K : never }[SettingKey];
export interface RawInfo { camera: string; neutral: [number, number, number]; xyz_to_camera: number[][]; baseline_exposure: number; calibration: number[][][]; calibration_signature: string; analog_balance: [number, number, number]; as_shot: Point; profile_signature: string }
export interface DevelopedPreview { path: string; inline_image?: string; image_bytes?: number; info: { width: number; height: number; histogram: number[][]; clipped_shadows: number; clipped_highlights: number; elapsed_ms: number }; base: { width: number; height: number; source_width: number; source_height: number; source_kind: string; color_note: string; raw?: RawInfo; raster_wb_temperature?: number } }
export interface Recipe { settings: Settings; defaults: Settings; revision: number; undo: Settings[] }
export interface HistoryRow { id: number; label: string; created: number }
export interface SavedRecipe { id: number; name: string; recipe: string }
export interface PreviewProgress { total: number; ready: number; generated: number; retained: number; failed: number; cancelled: boolean; paused: boolean; error: string | null; last_ready_id: number | null }
export interface ScanProgress { visited: number; indexed: number; unchanged: number; missing: number; errors: string[]; cancelled: boolean }
export interface ImportReport extends ScanProgress { previews: PreviewProgress }
export interface IndexProgress { processed: number; failed: number; remaining: boolean }
export interface Facet { value: string; count: number }
export type WorkflowRequest = { action: 'list'; item_id: number | null } | { action: 'store'; item_id: number | null; name: string; settings: Settings; fields: string[] } | { action: 'rename'; id: number; name: string } | { action: 'delete'; id: number } | { action: 'rename_history'; item_id: number; id: number; name: string } | { action: 'apply'; ids: number[]; settings: Settings; fields: string[] };
export type WorkflowResult<R extends WorkflowRequest> = R['action'] extends 'list' ? SavedRecipe[] : R['action'] extends 'store' ? number : R['action'] extends 'apply' ? number : null;
export interface UiState { selected_ids: number[]; active_id: number | null; view: ViewMode | 'develop'; query: Query; unsaved: { annotations: boolean; develop: boolean } }
export interface McpRequest { name: string; arguments: { ids: number[]; view: ViewMode | 'develop' } }
export interface McpResult { updated_ids?: number[]; ui_updated?: boolean; ui_error?: string; [key: string]: unknown }
export interface McpJob { kind: string; status: string; error?: string }
export type ErrorHandler = (failure: unknown) => void;
export type Edit = (change: (settings: Settings) => void, label?: string) => void;

import type { Photo, Root, Folder, Query, Page, Cursor, Detail, Preview, PhotoMetadata, Settings, SettingsPatch, Recipe, DevelopedPreview, HistoryRow, ImportReport, PreviewProgress, IndexProgress, Facet, WorkflowRequest, WorkflowResult, SavedRecipe, McpRequest, McpResult } from './models';
export type Language = 'ja' | 'en';
export interface AvailableUpdate { version: string; notes: string }
export type ExportFormat = 'jpeg' | 'png' | 'tiff';
export type CatalogRemoval = { kind: 'photos'; ids: number[] } | { kind: 'folder'; root_id: number; path: string };

export interface ExportRequest {
  ids: number[];
  folder: string;
  naming: string;
  format: ExportFormat;
  options: {
    quality: number;
    long_edge: number;
    max_bytes: number;
    border: { color: 'white' | 'black'; aspect_ratio: number } | null;
  };
}

export interface ExportReport {
  completed: number;
  total: number;
  cancelled: boolean;
  errors: string[];
  folder: string;
}

export interface McpStatus {
  enabled: boolean;
  running: boolean;
  port: number;
  url: string;
  error: string | null;
}

export interface Commands {
  app_version: { args: undefined; result: string };
  check_app_update: { args: undefined; result: AvailableUpdate | null };
  install_app_update: { args: undefined; result: void };
  roots: { args: undefined; result: Root[] };
  folders: { args: undefined; result: Folder[] };
  remove_from_catalog: { args: { selection: CatalogRemoval }; result: number };
  search: { args: { query: Query; cursor: Cursor | null }; result: Page };
  count: { args: { query: Query }; result: number };
  detail: { args: { id: number }; result: Detail };
  annotate: { args: { ids: number[]; annotation: { tags: string[]; rating: number; memo: string } }; result: void };
  set_photo_attributes: { args: { ids: number[]; rating: number | null; flag: number | null; label: string | null }; result: void };
  catalog_path: { args: undefined; result: string };
  catalog_needs_creation: { args: undefined; result: boolean };
  choose_catalog: { args: { create: boolean }; result: string | null };
  restart_catalog: { args: undefined; result: void };
  reveal_catalog: { args: undefined; result: void };
  save_catalog_copy: { args: undefined; result: string | null };
  choose_folder: { args: undefined; result: Root[] };
  scan: { args: { rootId: number | null }; result: ImportReport };
  cancel_scan: { args: undefined; result: void };
  prepare_library_previews: { args: { avifSupported: boolean }; result: PreviewProgress };
  set_preview_photo: { args: { id: number | null }; result: void };
  preview: { args: { id: number; large: boolean; bound?: number }; result: Preview };
  cancel_preview: { args: undefined; result: void };
  refresh_library_preview: { args: { id: number }; result: void };
  invalidate_library_preview: { args: { id: number; bound: number }; result: void };
  photo_metadata: { args: { id: number }; result: PhotoMetadata };
  reveal: { args: { id: number }; result: void };
  arm_close_guard: { args: undefined; result: void };
  close_window: { args: undefined; result: void };
  get_develop: { args: { id: number }; result: Recipe };
  save_develop: { args: { id: number; settings: Settings; expectedRevision: number; steps: { settings: Settings; label: string }[]; undo?: Settings[] }; result: number };
  develop_history: { args: { id: number; before: number | null }; result: HistoryRow[] };
  history_settings: { args: { id: number; historyId: number }; result: Settings };
  develop_preview: { args: { id: number; settings: Settings; bound: number }; result: DevelopedPreview };
  cancel_develop_preview: { args: undefined; result: void };
  set_develop_prefetch: { args: { ids: number[] }; result: void };
  develop_workflow: { args: { request: WorkflowRequest }; result: SavedRecipe[] | number | null };
  preset_file: { args: { write: boolean; value: SettingsPatch | null }; result: SettingsPatch | null };
  metadata_facets: { args: { query: Partial<Query>; field: string }; result: Facet[] };
  index_metadata_batch: { args: { query: Partial<Query> }; result: IndexProgress };
  cancel_metadata_index: { args: undefined; result: void };
  mcp_claim: { args: { id: string }; result: McpRequest };
  mcp_execute: { args: { id: string }; result: McpResult };
  mcp_complete: { args: { id: string; value: McpResult | null; error: string | null }; result: void };
  get_language: { args: undefined; result: Language };
  set_language: { args: { language: Language }; result: void };
  choose_export_folder: { args: undefined; result: string | null };
  cancel_batch_export: { args: undefined; result: void };
  export_develop_batch: { args: { request: ExportRequest }; result: ExportReport };
  mcp_status: { args: undefined; result: McpStatus };
  mcp_ready: { args: undefined; result: McpStatus };
  mcp_configure: { args: { enabled: boolean; port: number }; result: McpStatus };
  mcp_rotate: { args: undefined; result: McpStatus };
  mcp_token: { args: undefined; result: string };
}

export type Unlisten = () => void;
export interface BackendEvent<T> { payload: T }

declare global {
  interface Window {
    __TAURI__: {
      core: {
        invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
        convertFileSrc(path: string, protocol?: string): string;
      };
      event: {
        listen<T>(event: string, handler: (event: BackendEvent<T>) => void): Promise<Unlisten>;
      };
    };
  }
}

// The existing Tauri command names and serialized arguments are the backend contract.
export function command<K extends keyof Commands>(
  name: K,
  ...args: Commands[K]['args'] extends undefined ? [] : [Commands[K]['args']]
): Promise<Commands[K]['result']> {
  return window.__TAURI__.core.invoke(name, args[0]);
}

export type Invoke = typeof command;
export const invoke = command;

export function workflow<R extends WorkflowRequest>(request: R, invoke: Invoke = command): Promise<WorkflowResult<R>> {
  return invoke('develop_workflow', { request }) as Promise<WorkflowResult<R>>;
}

export function listen<T>(event: string, handler: (event: BackendEvent<T>) => void): Promise<Unlisten> {
  return window.__TAURI__.event.listen(event, handler);
}

export function convertFileSrc(path: string): string {
  return window.__TAURI__.core.convertFileSrc(path);
}

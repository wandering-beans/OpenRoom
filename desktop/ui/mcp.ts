import type { Invoke, listen as listenEvent, BackendEvent, Language } from './backend';
import type { UiState, ViewMode, McpResult, McpJob, ErrorHandler } from './models';
import { element as $ } from './dom';
import { t } from './i18n';
import { createPreferences } from './components/Preferences';
// MCP requests are acknowledged only after the desktop has applied the operation.
export async function startMcp({ invoke, listen, ready, state, prepare, select, view, refresh, busy, error, changeLanguage }: { invoke: Invoke; listen: typeof listenEvent; ready: boolean; state(): UiState; prepare(): Promise<void>; select(ids: number[]): Promise<void>; view(mode: ViewMode | 'develop'): Promise<void>; refresh(ids: number[]): Promise<void>; busy(): boolean; error: ErrorHandler; changeLanguage(value: Language): Promise<void> }) {

  let queue = Promise.resolve(), active = false, refreshPending = false;
  const preferences = createPreferences({ changeLanguage, error });
  async function handle(event: BackendEvent<{id: string}>) {
    const id = event.payload.id;
    let result: McpResult | null = null, failure: string | null = null, locked = false, priorInert = false;
    try {
      const request = await invoke('mcp_claim', { id });
      if (request.name !== 'get_app_state') {
        if (active || busy() || document.querySelector('dialog[open]')) throw Error(t("画面が操作中です。未保存の編集やダイアログを確認してください。"));
        active = true; priorInert = $('app-shell').inert; $('app-shell').inert = true; locked = true;
        await prepare();
      }
      // The backend checks the deadline again after flushing pending GUI edits.
      result = await invoke('mcp_execute', { id });
      if (request.name === 'get_app_state') result = { ...result, ...state() };
      else if (request.name === 'select_photos') { await select(request.arguments.ids); result = { ...result, ...state() }; }
      else if (request.name === 'set_view') { await view(request.arguments.view); result = { ...result, ...state() }; }
      else if (!request.name.startsWith('start_')) {
        try { await refresh(result.updated_ids || []); result.ui_updated = true; }
        catch (failure) { result.ui_updated = false; result.ui_error = String(failure); refreshPending = true; }
      }
    } catch (caught) { failure = String(caught); }
    finally {
      if (locked) { active = false; $('app-shell').inert = priorInert; }
      try { await invoke('mcp_complete', { id, value: result, error: failure }); }
      catch (caught) { if (!failure) error(caught); }
    }
  }
  await listen<{id: string}>('mcp-request', event => { queue = queue.then(() => handle(event)).catch(error); });
  await listen<McpJob>('mcp-job', event => {
    const job = event.payload;
    preferences.showJobStatus(`${job.kind}: ${job.status} ${job.error || ''}`);
    if (job.status !== 'running' && job.kind !== 'export') refreshPending = true;
  });
  // A completed background job must not discard manual metadata or in-progress gestures.
  setInterval(() => {
    if (!refreshPending || active || busy() || document.querySelector('dialog[open]')) return;
    refreshPending = false;
    queue = queue.then(async () => {
      if (active || busy()) { refreshPending = true; return; }
      const prior = $('app-shell').inert;
      active = true; $('app-shell').inert = true;
      try { await prepare(); await refresh([]); }
      catch { refreshPending = true; }
      finally { active = false; $('app-shell').inert = prior; }
    }).catch(error);
  }, 1000);
  preferences.showStatus(await invoke(ready ? 'mcp_ready' : 'mcp_status'));
}

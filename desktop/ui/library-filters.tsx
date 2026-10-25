import { createView } from './components/view';
import type { ReactNode } from 'react';
import type { Invoke } from './backend';
import type { Query, ErrorHandler, Facet } from './models';
export interface LibraryFilterState { flags: number[]; labels: string[]; extensions: string[]; metadata: Record<string,string[]>; rating_mode: string }
export interface LibraryFilterSnapshot { mode: string; filters: LibraryFilterState; columns: string[] }
import { element as $ } from './dom';
import { t } from './i18n';

const names: Record<string, string> = {
  date: t("日付"),
  camera: t("カメラ"),
  lens: t("レンズ"),
  label: t("ラベル"),
  iso: "ISO",
  aperture: t("絞り"),
  focal: t("焦点距離"),
  extension: t("ファイル形式"),
};
const colors: Record<string, string> = {
  red: t("レッド"),
  yellow: t("イエロー"),
  green: t("グリーン"),
  blue: t("ブルー"),
  purple: t("パープル"),
};
export function createLibraryFilters({
  invoke,
  query,
  refresh,
  error,
  resize,
}: { invoke: Invoke; query(): Query; refresh(): Promise<void>; error: ErrorHandler; resize(): void }) {
  let filters: LibraryFilterState = { flags: [], labels: [], extensions: [], metadata: {}, rating_mode: "gte" };
  let mode = "none",
    serial = 0,
    indexing = false,
    stop = false,
    restart = false,
    indexedScope: string | null = null;
  function currentScope() {
    const { root_id, folder } = query();
    return { root_id, folder };
  }
  const header = document.querySelector<HTMLElement>(".filter-heading")!;
  const text = $('filter-text'), attributes = $('attribute-filters'), metadata = $('metadata-filters');
  const headerView = createView(header), attributeView = createView(attributes), metadataView = createView(metadata);
  const columns = ['date', 'camera', 'lens', 'label'].map(field => ({ field, view: null as ReturnType<typeof createView> | null }));
  metadataView.render(columns.map((column, index) => <section className="metadata-column" key={index}>
    <select aria-label={t('メタデータ列')} defaultValue={column.field} onChange={event => {
      const previous = column.field;
      column.field = event.currentTarget.value;
      if (!columns.some(other => other !== column && other.field === previous)) {
        if (previous === 'label') filters.labels = [];
        else if (previous === 'extension') filters.extensions = [];
        else delete filters.metadata[previous];
      }
      changed();
    }}>{Object.entries(names).map(([key, label]) => <option key={key} value={key}>{label}</option>)}</select>
    <div className="metadata-values" ref={node => { if (node && !column.view) column.view = createView(node); }} />
  </section>));
  function renderHeader() {
    const custom = Object.values(filters.metadata).some(values => values.length) || filters.flags.length || filters.labels.length || filters.extensions.length || $('rating-filter').value !== '' || $('search').value;
    headerView.render(<>
      <strong>{t('ライブラリフィルター：')}</strong><nav id="filter-tabs" aria-label={t('フィルターの種類')}>
        {(['text', 'attribute', 'metadata', 'none'] as const).map((key, index) => <button key={key} data-mode={key} aria-pressed={mode === key} onClick={() => setMode(key)}>{[t('テキスト'), t('属性'), t('メタデータ'), t('なし')][index]}</button>)}
      </nav><span id="filter-summary">{custom ? t('カスタムフィルター') : t('フィルターなし')}</span>
      <div id="metadata-index-status" hidden={mode !== 'metadata'}><span id="metadata-index-message" />
        <button id="metadata-index" aria-label={t('メタデータを更新')} title={t('メタデータを更新')} onClick={() => { void index(); }}>↻</button>
        <button id="metadata-index-cancel" hidden onClick={() => { stop = true; restart = false; invoke('cancel_metadata_index').catch(error); }}>{t('中止')}</button>
      </div>
    </>);
  }
  function renderAttributes() {
    const rating = $('rating-filter').value;
    attributeView.render(<>
      <span>{t('フラグ：')}</span><div id="flag-filters">{([1, 0, -1] as const).map((value, index) => {
        const label = [t('採用'), t('フラグなし'), t('除外')][index];
        return <button key={value} title={label} aria-label={label + t('で絞り込み')} data-flag={value} aria-pressed={filters.flags.includes(value)} onClick={() => { toggle(filters.flags, value); changed(); }}>
          <svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d={value === -1 ? "M5 21V3m0 1c5-4 9 4 14 0v7M14 14l7 7m0-7-7 7" : "M5 21V3m0 1c5-4 9 4 14 0v10c-5 4-9-4-14 0"} fill={value === 1 ? "currentColor" : undefined} /></svg>
        </button>;
      })}</div>
      <span>{t('評価：')}</span><select id="rating-mode" aria-label={t('評価の条件')} value={filters.rating_mode} onChange={event => { filters.rating_mode = event.currentTarget.value; changed(); }}><option value="gte">≥</option><option value="eq">＝</option><option value="lte">≤</option></select>
      <div id="filter-stars" className="rating-stars">{Array.from({length: 6}, (_, value) => <button key={value} aria-label={t('評価{0}で絞り込み', value)} data-rating={value} className={rating !== '' && value <= Number(rating) ? 'rated' : ''} onClick={() => { $('rating-filter').value = String(value); changed(); }}>
        {value ? <svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9z" /></svg> : '0'}
      </button>)}</div><button id="rating-clear" onClick={() => { $('rating-filter').value = ''; changed(); }}>{t('解除')}</button>
      <span>{t('カラー：')}</span><div id="label-filters">{Object.entries(colors).map(([value, label]) => <button key={value} className="color-swatch" data-color={value} aria-label={label + t('で絞り込み')} title={label} aria-pressed={filters.labels.includes(value)} onClick={() => { toggle(filters.labels, value); changed(); }} />)}</div>
    </>);
  }
  function toggle<T>(list: T[], value: T) {
    const at = list.indexOf(value);
    if (at < 0) list.push(value);
    else list.splice(at, 1);
  }
  function sync() {
    renderHeader(); renderAttributes();
    text.hidden = mode !== 'text'; attributes.hidden = mode !== 'attribute'; metadata.hidden = mode !== 'metadata';
    resize();
  }
  function changed() {
    sync();
    refresh();
  }
  function setMode(value: string) {
    mode = value;
    if (value === "none") {
      clear();
      refresh();
    }
    sync();
    if (value === "metadata") {
      loadFacets();
      index();
    }
  }
  function clear() {
    mode = "none";
    filters = { flags: [], labels: [], extensions: [], metadata: {}, rating_mode: "gte" };
    $("rating-filter").value = "";
    $("search").value = "";
    sync();
  }
  async function loadFacets() {
    if (mode !== "metadata") return;
    const token = ++serial,
      request = query();
    try {
      const values = await Promise.all(
        columns.map((c) =>
          invoke("metadata_facets", { query: request, field: c.field }),
        ),
      );
      if (token !== serial) return;
      columns.forEach((column, index) => {
        const field = column.field,
          rows = values[index],
          selected =
            field === "label"
              ? filters.labels
              : field === "extension"
                ? request.extensions ?? []
                : filters.metadata[field] || [];
        function row(label: string, value: string | null, count: number, depth = 0) {
          return <button key={value ?? '__all'} className={`facet-row${value === null ? !selected.length ? ' selected' : '' : selected.includes(value) ? ' selected' : ''}`} style={{paddingLeft: `${9 + depth * 12}px`}} onClick={event => {
            const next = value !== null && field !== 'extension' && (event.metaKey || event.ctrlKey) ? [...selected] : [];
            if (value !== null) toggle(next, value);
            if (field === 'label') filters.labels = next;
            else if (field === 'extension') filters.extensions = next;
            else filters.metadata[field] = next;
            changed();
          }}><span>{label}</span><small>{count}</small></button>;
        }
        const content: ReactNode[] = [row(t('すべて（{0}項目）', rows.length), null, rows.reduce((sum, entry) => sum + entry.count, 0))];
        if (field === 'date') {
          const years = new Map<string, Facet[]>();
          for (const entry of rows) {
            const year = entry.value.slice(0, 4);
            if (!years.has(year)) years.set(year, []);
            years.get(year)!.push(entry);
          }
          for (const [year, dates] of years) content.push(<details key={year} open={selected.some(value => value.startsWith(year))}>
            <summary>{row(year ? t('{0}年', year) : t('日付不明'), year, dates.reduce((sum, entry) => sum + entry.count, 0))}</summary>
            {dates.map(entry => row(entry.value || t('日付不明'), entry.value, entry.count, 1))}
          </details>);
        } else for (const entry of rows) content.push(row(colors[entry.value] || entry.value || (field === 'label' ? t('ラベルなし') : t('不明')), entry.value, entry.count));
        column.view!.render(content);
      });
    } catch (failure) {
      if (token === serial) error(failure);
    }
  }
  async function index() {
    if (indexing) return;
    const scope = currentScope();
    indexedScope = JSON.stringify(scope);
    indexing = true;
    stop = false;
    restart = false;
    let processed = 0,
      failed = 0;
    $("metadata-index").disabled = true;
    $("metadata-index-cancel").hidden = false;
    try {
      let remaining = true;
      while (remaining && !stop) {
        const result = await invoke("index_metadata_batch", { query: scope });
        processed += result.processed;
        failed += result.failed;
        remaining = result.remaining;
        $("metadata-index-message").textContent =
          t("{0}枚の索引を更新{1}", processed, failed ? t("・{0}枚は取得不可", failed) : "");
        await loadFacets();
      }
      if (processed && !stop) await refresh();
      $("metadata-index-message").textContent = stop
        ? t("索引更新を中止しました")
        : t("索引更新済み{0}", failed ? t("（{0}枚は取得不可）", failed) : "");
    } catch (failure) {
      error(failure);
    } finally {
      indexing = false;
      $("metadata-index").disabled = false;
      $("metadata-index-cancel").hidden = true;
      if (restart && mode === "metadata") index();
    }
  }
  sync();
  return {
    clear,
    query: () => structuredClone(filters),
    snapshot: () => ({
      mode,
      filters: structuredClone(filters),
      columns: columns.map((c) => c.field),
    }),
    restore(value: LibraryFilterSnapshot | null) {
      if (value) {
        mode = value.mode;
        filters = structuredClone(value.filters);
        value.columns?.forEach((field, index) => {
          columns[index].field = field;
          metadata.querySelectorAll('select')[index].value = field;
        });
        sync();
        loadFacets();
      }
    },
    refresh() {
      sync();
      loadFacets();
      if (mode === "metadata" && indexedScope !== JSON.stringify(currentScope())) {
        if (indexing) {
          stop = true;
          restart = true;
          invoke("cancel_metadata_index").catch(error);
        } else index();
      }
    },
    text() {
      mode = "text";
      sync();
    },
    attribute() {
      mode = "attribute";
      sync();
    },
  };
}

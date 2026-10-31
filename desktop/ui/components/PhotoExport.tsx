import { useEffect, useImperativeHandle, useRef, useState, type Ref } from 'react';
import { createRoot } from 'react-dom/client';
import { createPortal, flushSync } from 'react-dom';
import { command, listen, type CatalogRemoval, type ExportFormat, type ExportReport, type Unlisten } from '../backend';
import { errorText, t } from '../i18n';

interface ExportActions {
  prepare(): Promise<boolean>;
  error(failure: unknown): void;
  remove(selection: CatalogRemoval): Promise<void>;
}
interface PhotoExportHandle {
  readonly busy: boolean;
  menu(selection: CatalogRemoval, x: number, y: number, tile: HTMLElement): void;
}
type Phase = 'idle' | 'choosing' | 'running';
const exportFolderKey = 'openroom.export.folder';

function PhotoExport({ prepare, error, remove, ref }: ExportActions & { ref: Ref<PhotoExportHandle> }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const browse = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef<HTMLElement | null>(null);
  const ids = useRef<number[]>([]);
  const [selection, setSelection] = useState<CatalogRemoval>({ kind: 'photos', ids: [] });
  const opening = useRef(false);
  const phaseRef = useRef<Phase>('idle');
  const [phase, setPhase] = useState<Phase>('idle');
  const [menuOpen, setMenuOpen] = useState(false);
  const [position, setPosition] = useState({ left: 0, top: 0 });
  const [count, setCount] = useState(0);
  const [format, setFormat] = useState<ExportFormat>('jpeg');
  const [border, setBorder] = useState<'none' | 'white' | 'black'>('none');
  const [folder, setFolder] = useState(() => {
    try { return localStorage.getItem(exportFolderKey) || ''; }
    catch { return ''; }
  });
  const [result, setResult] = useState('');
  const [cancelling, setCancelling] = useState(false);

  function changePhase(value: Phase) {
    phaseRef.current = value;
    setPhase(value);
  }
  function focusPhoto() {
    const target = returnFocus.current?.isConnected ? returnFocus.current : document.getElementById('filmstrip');
    target?.focus({ preventScroll: true });
  }
  function closeMenu(restoreFocus = false) {
    setMenuOpen(false);
    if (restoreFocus) focusPhoto();
  }
  useImperativeHandle(ref, () => ({
    get busy() { return dialog.current?.open ?? false; },
    menu(selection, x, y, tile) {
      if (opening.current || dialog.current?.open) return;
      ids.current = selection.kind === 'photos' ? [...selection.ids] : [];
      returnFocus.current = tile;
      flushSync(() => { setSelection(selection); setMenuOpen(true); });
      const element = menu.current!;
      flushSync(() => setPosition({
        left: Math.max(0, Math.min(x, window.innerWidth - element.offsetWidth)),
        top: Math.max(0, Math.min(y, window.innerHeight - element.offsetHeight)),
      }));
      element.querySelector<HTMLButtonElement>('button:not([hidden])')?.focus();
    },
  }));

  useEffect(() => {
    const pointerDown = (event: PointerEvent) => {
      if (!menu.current?.contains(event.target as Node)) setMenuOpen(false);
    };
    const keyDown = (event: KeyboardEvent) => {
      // Keyboard context menus can leave focus on the photo on Linux.
      if (menu.current && !menu.current.hidden && (event.key === 'Escape' || event.key === 'Tab')) {
        event.preventDefault(); event.stopPropagation(); closeMenu(true);
      }
    };
    const blur = () => setMenuOpen(false);
    const beforeUnload = (event: BeforeUnloadEvent) => {
      if (phaseRef.current === 'running') { event.preventDefault(); event.returnValue = ''; }
    };
    let disposed = false;
    let unlisten: Unlisten | undefined;
    listen<ExportReport>('develop-export-progress', ({ payload }) => {
      if (phaseRef.current === 'running') setResult(t('{0}/{1}枚を書き出し中…', payload.completed, payload.total));
    }).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(error);
    document.addEventListener('pointerdown', pointerDown);
    document.addEventListener('keydown', keyDown, true);
    window.addEventListener('blur', blur);
    window.addEventListener('beforeunload', beforeUnload);
    return () => {
      disposed = true;
      unlisten?.();
      document.removeEventListener('pointerdown', pointerDown);
      document.removeEventListener('keydown', keyDown, true);
      window.removeEventListener('blur', blur);
      window.removeEventListener('beforeunload', beforeUnload);
    };
  }, [error]);

  async function open() {
    closeMenu();
    if (opening.current || dialog.current?.open) return;
    const selection = [...ids.current];
    opening.current = true;
    try {
      if (!await prepare()) return;
      ids.current = selection;
      flushSync(() => { setCount(selection.length); setResult(''); });
      dialog.current?.showModal();
    } catch (failure) { error(failure); }
    finally { opening.current = false; }
  }
  async function chooseFolder() {
    if (phaseRef.current !== 'idle') return;
    // Finish WebView focus changes before the native sheet takes focus.
    flushSync(() => changePhase('choosing'));
    try {
      const chosen = await command('choose_export_folder');
      if (chosen) {
        setFolder(chosen);
        try { localStorage.setItem(exportFolderKey, chosen); }
        catch { /* Export still works when local preferences cannot be saved. */ }
      }
    } catch (failure) { setResult(errorText(failure)); }
    finally { changePhase('idle'); }
  }
  async function cancel() {
    setCancelling(true);
    try {
      await command('cancel_batch_export');
      if (phaseRef.current === 'running') setResult(t('書き出しを中止しています…'));
    } catch (failure) { setResult(errorText(failure)); setCancelling(false); }
  }
  async function submit(form: HTMLFormElement) {
    if (phaseRef.current !== 'idle') return;
    if (!folder) {
      setResult(t('書き出し先のフォルダを選択してください。'));
      browse.current?.focus();
      return;
    }
    const values = new FormData(form);
    changePhase('running');
    setCancelling(false);
    setResult(t('0/{0}枚を書き出し中…', ids.current.length));
    try {
      const report = await command('export_develop_batch', { request: {
        ids: [...ids.current], folder, format, naming: String(values.get('naming')),
        options: {
          quality: format === 'jpeg' ? Number(values.get('quality')) : 92,
          long_edge: Number(values.get('long_edge')),
          max_bytes: Number(values.get('max_kb')) * 1024,
          border: border === 'none' ? null : { color: border, aspect_ratio: Number(values.get('aspect_ratio')) },
        },
      } });
      setResult(t('{0}/{1}枚を書き出しました{2}。\n{3}', report.completed, report.total,
        report.cancelled ? t('（中止）') : '', report.errors.join('\n')));
    } catch (failure) { setResult(errorText(failure)); }
    finally { changePhase('idle'); }
  }

  return <>
    <div id="photo-menu" ref={menu} role="menu" aria-label={selection.kind === 'photos' ? t('写真の操作') : t('フォルダの操作')} hidden={!menuOpen} style={position}
      onKeyDown={event => {
        if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault();
        const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not([hidden])')];
        const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
        buttons[event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowUp' ? -1 : 1) + buttons.length) % buttons.length]?.focus();
      }}>
      <button id="photo-export" role="menuitem" hidden={selection.kind !== 'photos'} onClick={open}>{t('書き出し…')}</button>
      <button id="catalog-remove" role="menuitem" onClick={() => { closeMenu(); void remove(selection).catch(error).finally(focusPhoto); }}>{t('カタログから削除…')}</button>
    </div>
    <dialog id="export-dialog" ref={dialog} aria-labelledby="export-title" onClose={focusPhoto}
      onCancel={event => { if (phaseRef.current !== 'idle') event.preventDefault(); }}>
      <form id="export-form" onSubmit={event => { event.preventDefault(); void submit(event.currentTarget); }}>
        <h2 id="export-title">{t('写真を書き出し')}</h2>
        <p id="export-selection">{t('{0}枚の写真を書き出します。', count)}</p>
        <fieldset id="export-settings" disabled={phase === 'running'}>
          <label>{t('ファイル形式')}<select id="export-format" value={format} onChange={event => setFormat(event.target.value as ExportFormat)}>
            <option value="jpeg">JPEG</option><option value="png">PNG 16bit</option><option value="tiff">TIFF 16bit</option>
          </select></label>
          <label>{t('書き出し画質（JPEG、1〜100）')}<input id="export-quality" name="quality" type="number" min="1" max="100" defaultValue="100" required disabled={format !== 'jpeg'} /></label>
          <label>{t('容量上限（1枚あたりのKB、0＝制限なし）')}<input id="export-max-kb" name="max_kb" type="number" min="0" max="1048576" defaultValue="0" required /></label>
          <p className="panel-note">{t('1 KB＝1,024バイト。JPEGは指定画質を上限に、容量に合わせて画質を下げます。PNG・TIFFは画質を保持します。上限に収まらない写真は保存せず、結果に表示します。')}</p>
          <label>{t('長辺（px、0＝元の大きさ）')}<input id="export-long-edge" name="long_edge" type="number" min="0" max="65535" defaultValue="0" required /></label>
          <label>{t('余白の枠')}<select id="export-border" value={border} onChange={event => setBorder(event.target.value as typeof border)}>
            <option value="none">{t('枠なし')}</option><option value="white">{t('白枠')}</option><option value="black">{t('黒枠')}</option>
          </select></label>
          <label hidden={border === 'none'}>{t('枠を含む画像の比率（横：縦）')}<select id="export-border-ratio" name="aspect_ratio" defaultValue="0" disabled={border === 'none'}>
            <option value="0">{t('写真と同じ比率')}</option>
            <option value="1">1:1</option>
            <option value={3 / 2}>3:2</option><option value={2 / 3}>2:3</option>
            <option value={4 / 3}>4:3</option><option value={3 / 4}>3:4</option>
            <option value={5 / 4}>5:4</option><option value={4 / 5}>4:5</option>
            <option value={16 / 9}>16:9</option><option value={9 / 16}>9:16</option>
          </select></label>
          <p className="panel-note" hidden={border === 'none'}>{t('写真を切り抜かず中央に配置し、四辺に短辺の約4％以上の余白を加えます。長辺の指定は枠を含むサイズに適用します。0の場合は写真を縮小せず、枠の分だけ画像が大きくなります。')}</p>
          <label>{t('書き出し先フォルダ')}<input id="export-folder" placeholder={t('フォルダを選択してください')} value={folder} readOnly /></label>
          <button id="export-browse" type="button" ref={browse} onClick={chooseFolder} disabled={phase !== 'idle'}>{t('フォルダを選択…')}</button>
          <label>{t('命名規則（拡張子は自動で追加）')}<input id="export-naming" name="naming" defaultValue="{name}-edited" maxLength={200} required /></label>
          <p className="panel-note">{t('{name}＝元の名前、{seq}＝4桁の連番、{id}＝写真ID。同名ファイルがある場合は末尾に番号を付けます。')}</p>
        </fieldset>
        <p id="export-result" role="status" aria-live="polite">{result}</p>
        <div className="export-actions">
          <button id="export-close" type="button" disabled={phase !== 'idle'} onClick={() => dialog.current?.close()}>{t('閉じる')}</button>
          <button id="export-cancel" type="button" hidden={phase !== 'running'} disabled={cancelling} onClick={cancel}>{t('書き出しを中止')}</button>
          <button id="export-start" type="submit" disabled={phase !== 'idle'}>{t('書き出し')}</button>
        </div>
      </form>
    </dialog>
  </>;
}

export function createPhotoExport(actions: ExportActions): PhotoExportHandle {
  const container = document.getElementById('photo-export-root');
  if (!container) throw new Error('Missing export root');
  const handle = { current: null as PhotoExportHandle | null };
  // Keep modal dialogs outside layout wrappers, as in the original native WebView page.
  flushSync(() => createRoot(container).render(createPortal(<PhotoExport {...actions} ref={handle} />, document.body)));
  return {
    get busy() { return handle.current!.busy; },
    menu: (...args) => handle.current!.menu(...args),
  };
}

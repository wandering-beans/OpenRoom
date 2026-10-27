import { Fragment, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type MouseEvent, type KeyboardEvent } from 'react';
import { t, metadataValue } from '../i18n';
import type { Photo, Preview, Root, PhotoMetadata } from '../models';

export interface ThumbnailLoader {
  (photo: Photo, complete: (preview: Preview) => void, failed: (error: unknown) => void, alive: () => boolean, repair: boolean): void;
}
interface TileProps {
  photo: Photo; index: number; film: boolean; selected: boolean; active: boolean; adjusted: boolean; style: CSSProperties;
  source(path: string): string; load: ThumbnailLoader; sizeLabel: string;
  select(event?: { metaKey?: boolean; ctrlKey?: boolean; shiftKey?: boolean }): Promise<boolean>;
  menu(event: MouseEvent | KeyboardEvent): void; open(): Promise<void>;
  element(node: HTMLButtonElement | null): void;
}
export function PhotoTile({ photo, index, film, selected, active, adjusted, style, source, load, sizeLabel, select, menu, open, element }: TileProps) {
  const flagLabel = photo.flag === -1 ? t('除外') : photo.flag === 1 ? t('採用') : '';
  const ratingLabel = photo.rating ? `${t('評価')} ${photo.rating}/5` : '';
  const [preview, setPreview] = useState<Preview | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [failure, setFailure] = useState('');
  const [attempt, setAttempt] = useState(0);
  const thumb = useRef<HTMLDivElement>(null), image = useRef<HTMLImageElement>(null), badge = useRef<SVGSVGElement>(null);
  useLayoutEffect(() => {
    if (!adjusted || !loaded) return;
    function positionBadge() {
      if (!thumb.current || !image.current || !badge.current) return;
      const frame = thumb.current.getBoundingClientRect(), photo = image.current.getBoundingClientRect();
      badge.current.style.right = `${frame.right - photo.right + 3}px`;
      badge.current.style.bottom = `${frame.bottom - photo.bottom + 3}px`;
    }
    const observer = new ResizeObserver(positionBadge);
    observer.observe(thumb.current!);
    observer.observe(image.current!);
    positionBadge();
    return () => observer.disconnect();
  }, [adjusted, loaded, preview?.path]);
  useEffect(() => {
    let mounted = true;
    load(photo, result => { if (mounted) { setPreview(result); setFailure(''); } }, error => {
      if (!mounted) return;
      setFailure(String(error));
      if (!attempt) setAttempt(1);
    }, () => mounted, attempt > 0);
    return () => { mounted = false; };
  }, [photo.id, photo.modified, photo.size, attempt, load]);
  return <button ref={element} className={`${film ? 'film-tile' : 'tile'}${selected ? ' selected' : ''}${active ? ' active-photo' : ''}${photo.flag === -1 ? ' rejected' : ''}`}
    aria-label={[`${film ? t('フィルム ') : ''}${photo.name}`, sizeLabel, flagLabel, ratingLabel, adjusted ? t('調整済み') : ''].filter(Boolean).join('、')} aria-pressed={selected} aria-haspopup="menu" title={photo.name} data-label={photo.label || ''} style={style}
    onClick={event => { void select(event); }} onContextMenu={menu} onDoubleClick={() => { void open(); }}
    onKeyDown={event => { if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) { event.stopPropagation(); menu(event); } }}>
    <span className="tile-heading">
      {flagLabel && <svg className={`photo-flag ${photo.flag === -1 ? 'flag-rejected' : 'flag-picked'}`} viewBox="0 0 24 24" role="img" aria-label={flagLabel}>
        <title>{flagLabel}</title>
        <path d="M5 21V3m0 1c5-4 9 4 14 0v10c-5 4-9-4-14 0" />
        {photo.flag === -1 && <path className="flag-cross" d="m9 6 6 6m0-6-6 6" />}
      </svg>}
      <span className={film ? 'film-number' : 'grid-number'}>{index + 1}</span>
    </span>
    <div ref={thumb} className="thumb" title={failure || undefined}>
      {preview && <img ref={image} src={source(preview.path)} alt="" decoding="async" onLoad={() => setLoaded(true)} onError={() => {
        setPreview(null); setLoaded(false); setFailure(t('プレビュー不可')); if (!attempt) setAttempt(1);
      }} />}
      {!loaded && <span className="placeholder">{failure ? t('プレビュー不可') : photo.extension.toUpperCase()}</span>}
      {!film && <span className="badge">{photo.extension.toUpperCase()}</span>}
      {adjusted && <svg ref={badge} className="adjustment-badge" viewBox="0 0 24 24" role="img" aria-label={t('調整済み')}>
        <title>{t('調整済み')}</title>
        <path d="M3 3h18v18H3zM4 20 20 4M6 9h6M9 6v6M13 16h5" />
      </svg>}
    </div>
    {!film && <div className="tile-name">{photo.name}</div>}
    <span className="tile-rating" role={photo.rating ? 'img' : undefined} aria-label={ratingLabel || undefined} title={ratingLabel || undefined}>{'★'.repeat(photo.rating)}</span>
  </button>;
}

export interface FolderNode { path: string; name: string; count: number; children: Map<string, FolderNode> }
interface FolderProps { root: Root; node: FolderNode; depth: number; filter: string; rootId: number | null; folderPath: string | null; choose(root: Root, node: FolderNode): void }
function matches(node: FolderNode, filter: string): boolean {
  return !filter || node.name.toLocaleLowerCase().includes(filter) || [...node.children.values()].some(child => matches(child, filter));
}
export function FolderBranch({ root, node, depth, filter, rootId, folderPath, choose }: FolderProps) {
  const [expanded, setExpanded] = useState(depth === 0 || Boolean(filter) || (rootId === root.id && Boolean(folderPath?.startsWith(node.path))));
  if (!matches(node, filter)) return null;
  const childFilter = !filter || node.name.toLocaleLowerCase().includes(filter) ? '' : filter;
  const children = [...node.children.values()].filter(child => matches(child, childFilter));
  return <div className="folder-branch">
    <div className={`folder-row${rootId === root.id && folderPath === node.path ? ' active' : ''}`} style={{ '--depth': depth } as CSSProperties} data-root={root.id} data-folder={node.path}>
      <button className="folder-toggle" aria-label={t('{0}のサブフォルダーを切り替え', node.name)} disabled={!children.length} aria-expanded={expanded} onClick={() => setExpanded(!expanded)}>{children.length ? expanded ? '▾' : '▸' : '·'}</button>
      <button className="folder-label" title={node.path ? root.path + (root.path.includes('\\') ? '\\' : '/') + node.path : root.path} onClick={() => { choose(root, node); setExpanded(true); }}><span className="folder-icon" /><span>{node.name}</span></button>
      <span className="folder-count">{node.count.toLocaleString()}</span>
    </div>
    <div hidden={!expanded}>{children.map(child => <FolderBranch key={child.path} root={root} node={child} depth={depth + 1} filter={childFilter} rootId={rootId} folderPath={folderPath} choose={choose} />)}</div>
  </div>;
}
export function Properties({ fields }: { fields: PhotoMetadata }) {
  return Object.entries(fields).map(([name, value]) => <Fragment key={name}><dt>{t(name)}</dt><dd>{metadataValue(name, value)}</dd></Fragment>);
}

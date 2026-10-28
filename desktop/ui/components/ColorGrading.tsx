import { useImperativeHandle, useRef, useState, type ReactNode, type Ref } from 'react';
import type { SliderHandle } from './AdjustmentSlider';
import { t } from '../i18n';

interface Props {
  ref: Ref<SliderHandle>;
  values(): number[][];
  blocked(): boolean;
  begin(owner: HTMLElement): void;
  change(zone: number, hue: number, saturation: number): void;
  finish(): void;
  reset(zone: number): void;
  sliders: ReactNode[][];
  blending: ReactNode;
  balance: ReactNode;
}
const labels = ['シャドウ', '中間調', 'ハイライト', '全体'];

export function ColorGrading(props: Props) {
  const [mode, setMode] = useState(-1);
  const [values, setValues] = useState(props.values);
  const pointer = useRef<number | null>(null);
  useImperativeHandle(props.ref, () => ({ sync() { setValues(props.values()); } }), []);

  function move(zone: number, element: HTMLButtonElement, x: number, y: number) {
    if (props.blocked()) return;
    const box = element.getBoundingClientRect();
    const dx = (x - box.left - box.width / 2) / (box.width / 2);
    const dy = (box.top + box.height / 2 - y) / (box.height / 2);
    const saturation = Math.min(100, Math.round(Math.hypot(dx, dy) * 100));
    // Keep the chosen hue at the neutral center so increasing saturation restores it.
    const hue = saturation === 0 ? props.values()[zone][0] : (Math.round(Math.atan2(dy, dx) * 180 / Math.PI) + 360) % 360;
    props.change(zone, hue, saturation);
  }
  function release() {
    if (pointer.current === null) return;
    pointer.current = null;
    props.finish();
  }
  return <div className={`color-grading ${mode === -1 ? 'grading-three' : 'grading-single'}`}>
    <div className="grading-modes" role="group" aria-label={t('カラーグレーディングの範囲')}>
      <span>{t('色調補正:')}</span>
      {[-1, 0, 1, 2, 3].map(zone => <button type="button" key={zone} data-grading-mode={zone} aria-label={t(zone === -1 ? '3方向' : labels[zone])}
        title={t(zone === -1 ? '3方向' : labels[zone])} aria-pressed={mode === zone} onClick={() => { props.finish(); setMode(zone); }}>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          {zone === -1 ? <g fill="none" stroke="currentColor" strokeWidth="2.4"><circle cx="12" cy="6" r="3.5"/><circle cx="6" cy="17" r="3.5"/><circle cx="18" cy="17" r="3.5"/></g>
            : <><circle cx="12" cy="12" r="8" fill={['#242424', '#888', '#ddd', 'none'][zone]} stroke="currentColor" strokeWidth="2"/>{zone === 3 && <path d="M12 4a8 8 0 0 1 0 16Z" fill="currentColor"/>}</>}
        </svg>
      </button>)}
    </div>
    <div className="grading-wheels">
      {[1, 0, 2, 3].map(zone => {
        const [hue, saturation] = values[zone];
        const angle = hue * Math.PI / 180;
        return <section key={zone} className={`grading-zone grading-zone-${zone}`} data-grading={zone} hidden={mode === -1 ? zone === 3 : zone !== mode} aria-label={t(labels[zone])}>
          <h3>{t(labels[zone])}</h3>
          <div className="grading-wheel-row">
            <button type="button" className="grading-wheel" data-wheel={zone}
              aria-label={t('{0}の色相と彩度。左右キーで色相、上下キーで彩度を調整', t(labels[zone]))}
              title={t('色相 {0}°・彩度 {1}。ダブルクリックでリセット', hue, saturation)}
              onPointerDown={event => {
                if (event.button !== 0 || props.blocked()) return;
                event.preventDefault(); event.currentTarget.focus();
                pointer.current = event.pointerId; event.currentTarget.setPointerCapture(event.pointerId);
                props.begin(event.currentTarget); move(zone, event.currentTarget, event.clientX, event.clientY);
              }}
              onPointerMove={event => { if (pointer.current === event.pointerId) move(zone, event.currentTarget, event.clientX, event.clientY); }}
              onPointerUp={release} onPointerCancel={release} onLostPointerCapture={release}
              onBlur={() => { release(); props.finish(); }}
              onDoubleClick={() => { if (!props.blocked()) props.reset(zone); }}
              onKeyDown={event => {
                if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
                event.preventDefault(); event.stopPropagation();
                if (props.blocked()) return;
                props.begin(event.currentTarget);
                const [h, s] = props.values()[zone], step = event.shiftKey ? 10 : 1;
                const nextHue = (h + (event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0) + 360) % 360;
                const nextSaturation = event.key === 'Home' ? 0 : event.key === 'End' ? 100 : Math.max(0, Math.min(100, s + (event.key === 'ArrowUp' ? step : event.key === 'ArrowDown' ? -step : 0)));
                props.change(zone, nextHue, nextSaturation);
              }} onKeyUp={event => { if (event.key.startsWith('Arrow') || ['Home', 'End'].includes(event.key)) { event.stopPropagation(); props.finish(); } }}>
              <span className="grading-wheel-center" />
              <span className="grading-wheel-handle" style={{ left: `${50 + Math.cos(angle) * saturation * .5}%`, top: `${50 - Math.sin(angle) * saturation * .5}%` }} />
            </button>
            <span className="grading-swatch" aria-hidden="true" style={{ background: `hsl(${hue} ${saturation}% 50%)` }} />
          </div>
          <div className="grading-hue-saturation" hidden={mode === -1}>{props.sliders[zone].slice(0, 2)}</div>
          <div className="grading-luminance">{props.sliders[zone][2]}</div>
          <button type="button" className="grading-reset" aria-label={t('{0}のカラーグレーディングをリセット', t(labels[zone]))} onClick={() => props.reset(zone)}>{t('リセット')}</button>
        </section>;
      })}
    </div>
    <div className="grading-mix">{props.blending}{props.balance}</div>
  </div>;
}

import { useEffect, useImperativeHandle, useRef, useState, type CSSProperties, type Ref } from 'react';
import { t } from '../i18n';

export interface SliderState { value: number; disabled: boolean; hidden: boolean; track?: string }
export interface SliderHandle { sync(): void }
interface Props {
  id: string; label: string; title: string; min: number; max: number; step: number; kelvin: boolean; local: boolean; angle: boolean;
  state(): SliderState; prepare(input: HTMLInputElement): void; input(value: number, element: HTMLInputElement, number: boolean): void;
  finish(input: HTMLInputElement): void; reset(): void; sync(): void; levelGrid(show: boolean): void; ref: Ref<SliderHandle>;
}
export function AdjustmentSlider(props: Props) {
  const range = useRef<HTMLInputElement>(null), number = useRef<HTMLInputElement>(null);
  const [state, setState] = useState(props.state);
  const [text, setText] = useState(String(state.value));
  useImperativeHandle(props.ref, () => ({ sync() {
    const next = props.state(); setState(next);
    if (document.activeElement !== number.current) setText(props.angle ? next.value.toFixed(2) : String(next.value));
  } }), []);
  useEffect(() => {
    const inputs = [range.current!, number.current!];
    const finish = (event: Event) => props.finish(event.currentTarget as HTMLInputElement);
    const release = () => props.levelGrid(false);
    for (const input of inputs) input.addEventListener('change', finish);
    if (props.angle) for (const event of ['pointerup', 'pointercancel', 'blur']) window.addEventListener(event, release);
    return () => {
      for (const input of inputs) input.removeEventListener('change', finish);
      if (props.angle) for (const event of ['pointerup', 'pointercancel', 'blur']) window.removeEventListener(event, release);
    };
  }, []);
  const rangeValue = props.kelvin ? Math.log(state.value / props.min) / Math.log(props.max / props.min) * 1000 : state.value;
  function change(element: HTMLInputElement, numeric: boolean) {
    if (numeric) setText(element.value);
    const value = !numeric && props.kelvin ? Math.round(props.min * (props.max / props.min) ** (Number(element.value) / 1000)) : Number(element.value);
    if (element.value !== '' && Number.isFinite(value) && value >= props.min && value <= props.max) props.input(value, element, numeric);
  }
  function finish(element: HTMLInputElement) { props.finish(element); if (props.angle) props.levelGrid(false); }
  return <label className="develop-slider" title={props.title} hidden={state.hidden}>
    <span>{props.label}</span>
    <input ref={range} id={props.id} type="range" min={props.kelvin ? 0 : props.min} max={props.kelvin ? 1000 : props.max} step={props.kelvin ? 1 : props.step}
      value={Number.isFinite(rangeValue) ? rangeValue : 0} disabled={state.disabled} style={{ '--range-track': state.track } as CSSProperties}
      aria-label={(props.local ? t('マスクの') : '') + props.label} aria-valuetext={props.kelvin ? `${state.value} K` : undefined}
      onFocus={event => props.prepare(event.currentTarget)} onPointerDown={event => { props.prepare(event.currentTarget); if (props.angle) props.levelGrid(true); }}
      onInput={event => change(event.currentTarget, false)} onChange={() => {}}
      onPointerUp={event => finish(event.currentTarget)} onPointerCancel={event => finish(event.currentTarget)}
      onBlur={event => { finish(event.currentTarget); props.sync(); }}
      onKeyDown={event => { if (props.angle && ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown'].includes(event.key)) props.levelGrid(true); }}
      onKeyUp={event => finish(event.currentTarget)} onDoubleClick={event => { event.preventDefault(); props.reset(); }} />
    <input ref={number} id={`${props.id}-value`} type="number" min={props.min} max={props.max} step={props.step} value={text} disabled={state.disabled}
      aria-label={(props.local ? t('マスクの') : '') + props.label + t('の数値')}
      onFocus={event => props.prepare(event.currentTarget)} onPointerDown={event => props.prepare(event.currentTarget)}
      onInput={event => change(event.currentTarget, true)} onChange={() => {}}
      onPointerUp={event => finish(event.currentTarget)} onPointerCancel={event => finish(event.currentTarget)}
      onBlur={event => { finish(event.currentTarget); props.sync(); }} onDoubleClick={event => { event.preventDefault(); props.reset(); }} />
  </label>;
}

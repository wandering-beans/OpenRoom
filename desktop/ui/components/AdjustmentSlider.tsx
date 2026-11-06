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
  const rangePointer = useRef<number | null>(null);
  const rangeKeys = useRef(new Set<string>());
  const adjustmentKeys = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown'];
  const [state, setState] = useState(props.state);
  const [text, setText] = useState(String(state.value));
  useImperativeHandle(props.ref, () => ({ sync() {
    const next = props.state(); setState(next);
    if (document.activeElement !== number.current) setText(props.angle ? next.value.toFixed(2) : String(next.value));
  } }), []);
  useEffect(() => {
    const inputs = [range.current!, number.current!];
    const finishChange = (event: Event) => {
      // Chromium fires change on each keyboard step, before the held key is released.
      if (event.currentTarget === range.current && rangeKeys.current.size) return;
      finish(event.currentTarget as HTMLInputElement);
    };
    const releasePointer = (event: PointerEvent) => {
      if (event.pointerId === rangePointer.current) finish(range.current!);
    };
    const releaseWindow = () => { if (rangePointer.current !== null || rangeKeys.current.size) finish(range.current!); };
    const release = () => props.levelGrid(false);
    for (const input of inputs) input.addEventListener('change', finishChange);
    for (const event of ['pointerup', 'pointercancel'] as const) window.addEventListener(event, releasePointer);
    window.addEventListener('blur', releaseWindow);
    if (props.angle) for (const event of ['pointerup', 'pointercancel', 'blur']) window.addEventListener(event, release);
    return () => {
      for (const input of inputs) input.removeEventListener('change', finishChange);
      for (const event of ['pointerup', 'pointercancel'] as const) window.removeEventListener(event, releasePointer);
      window.removeEventListener('blur', releaseWindow);
      if (props.angle) for (const event of ['pointerup', 'pointercancel', 'blur']) window.removeEventListener(event, release);
    };
  }, []);
  const rangeValue = props.kelvin ? Math.log(state.value / props.min) / Math.log(props.max / props.min) * 1000 : state.value;
  function change(element: HTMLInputElement, numeric: boolean) {
    if (numeric) setText(element.value);
    const value = !numeric && props.kelvin ? Math.round(props.min * (props.max / props.min) ** (Number(element.value) / 1000)) : Number(element.value);
    if (element.value !== '' && Number.isFinite(value) && value >= props.min && value <= props.max) props.input(value, element, numeric);
  }
  function finish(element: HTMLInputElement) {
    if (element === range.current) { rangePointer.current = null; rangeKeys.current.clear(); }
    props.finish(element); if (props.angle) props.levelGrid(false);
  }
  function reset(event: React.MouseEvent<HTMLInputElement>) {
    event.preventDefault();
    props.reset();
    const value = props.state().value;
    setText(props.angle ? value.toFixed(2) : String(value));
  }
  return <label className="develop-slider" title={props.title} hidden={state.hidden}>
    <span>{props.label}</span>
    <input ref={range} id={props.id} type="range" min={props.kelvin ? 0 : props.min} max={props.kelvin ? 1000 : props.max} step={props.kelvin ? 1 : props.step}
      value={Number.isFinite(rangeValue) ? rangeValue : 0} disabled={state.disabled} style={{ '--range-track': state.track } as CSSProperties}
      aria-label={(props.local ? t('マスクの') : '') + props.label} aria-valuetext={props.kelvin ? `${state.value} K` : undefined}
      onFocus={event => props.prepare(event.currentTarget)} onPointerDown={event => {
        if (event.button !== 0) return;
        rangePointer.current = event.pointerId;
        props.prepare(event.currentTarget); if (props.angle) props.levelGrid(true);
      }}
      onInput={event => change(event.currentTarget, false)} onChange={() => {}}
      onPointerUp={event => finish(event.currentTarget)} onPointerCancel={event => finish(event.currentTarget)}
      onBlur={event => {
        // WebKit can blur a range immediately after mouse-down; the pointer still owns the drag.
        if (rangePointer.current === null) { finish(event.currentTarget); props.sync(); }
      }}
      onKeyDown={event => {
        if (!adjustmentKeys.includes(event.key)) return;
        rangeKeys.current.add(event.key);
        if (props.angle) props.levelGrid(true);
      }}
      onKeyUp={event => {
        if (rangeKeys.current.delete(event.key) && !rangeKeys.current.size) finish(event.currentTarget);
      }} onDoubleClick={reset} />
    <input ref={number} id={`${props.id}-value`} type="number" min={props.min} max={props.max} step={props.step} value={text} disabled={state.disabled}
      aria-label={(props.local ? t('マスクの') : '') + props.label + t('の数値')}
      onFocus={event => props.prepare(event.currentTarget)} onPointerDown={event => props.prepare(event.currentTarget)}
      onInput={event => change(event.currentTarget, true)} onChange={() => {}}
      onPointerUp={event => finish(event.currentTarget)} onPointerCancel={event => finish(event.currentTarget)}
      onBlur={event => { finish(event.currentTarget); props.sync(); }} onDoubleClick={reset} />
  </label>;
}

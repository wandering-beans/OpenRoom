import { createView } from './components/view';
import type { Point, Settings, Edit } from './models';
import { element as $ } from './dom';
import { t } from './i18n';

// Match the natural cubic spline used by the Rust point-curve renderer.
function secondDerivatives(points: Point[]) {
  const second: number[] = Array(points.length).fill(0), rhs: number[] = Array(points.length).fill(0);
  for (let i = 1; i < points.length - 1; i++) {
    const left = points[i][0] - points[i - 1][0];
    const right = points[i + 1][0] - points[i][0];
    const span = left + right, ratio = left / span;
    const pivot = ratio * second[i - 1] + 2;
    second[i] = (ratio - 1) / pivot;
    const slopeChange = (points[i + 1][1] - points[i][1]) / right
      - (points[i][1] - points[i - 1][1]) / left;
    rhs[i] = (6 * slopeChange / span - ratio * rhs[i - 1]) / pivot;
  }
  for (let i = points.length - 2; i >= 0; i--) second[i] = second[i] * second[i + 1] + rhs[i];
  return second;
}
export function curveValue(x: number, points: Point[], second = secondDerivatives(points)) {
  let i = 0;
  while (i < points.length - 2 && x > points[i + 1][0]) i++;
  const width = points[i + 1][0] - points[i][0];
  const b = Math.max(0, Math.min(1, (x - points[i][0]) / width)), a = 1 - b;
  return Math.max(0, Math.min(1, a * points[i][1] + b * points[i + 1][1]
    + ((a * a * a - a) * second[i] + (b * b * b - b) * second[i + 1]) * width * width / 6));
}
export function createToneCurve({ get, begin, change, end, edit, blocked }: { get(): Settings | null; begin(): void; change: (fn: (settings: Settings) => void) => void; end(): void; edit: Edit; blocked(): boolean }) {
  const svg = document.querySelector<SVGSVGElement>(".tone-curve")!;
  const handles = createView($('curve-handles'));
  let selected = 0,
    dragging = false;
  const channel = () => Number($("develop-curve-channel").value);
  function points(): Point[] {
    const s = get();
    if (!s)
      return [
        [0, 0],
        [1, 1],
      ];
    const saved = s.point_curves?.[channel()];
    if (saved?.length) return structuredClone(saved);
    const old = s.curves[channel()];
    return old.every((v, i) => v === i / 4)
      ? [
          [0, 0],
          [1, 1],
        ]
      : old.map((v, i) => [i / 4, v]);
  }
  function store(values: Point[]) {
    change((s) => {
      s.point_curves ??= [[], [], [], []];
      s.point_curves[channel()] = values;
    });
  }
  function draw() {
    const values = points();
    selected = Math.min(selected, values.length - 1);
    const legacy = !get()?.point_curves?.[channel()]?.length;
    const second = secondDerivatives(values);
    const path = legacy
      ? values.map((p) => [p[0] * 256, 256 - p[1] * 256])
      : Array.from({ length: 257 }, (_, x) => [
          x,
          256 - curveValue(x / 256, values, second) * 256,
        ]);
    $("develop-curve-line").setAttribute(
      "points",
      path.map((p) => p.join(",")).join(" "),
    );
    handles.render(values.map((point, index) => <circle key={index} cx={point[0] * 256} cy={256 - point[1] * 256} r={index === selected ? 5 : 4} data-point={index} className={index === selected ? 'selected' : ''} />));
    $("curve-input").value = String(Math.round(values[selected][0] * 255));
    $("curve-output").value = String(Math.round(values[selected][1] * 255));
    $("curve-input").disabled =
      selected === 0 || selected === values.length - 1;
    $("curve-delete").disabled =
      selected === 0 || selected === values.length - 1;
  }
  function coordinates(event: MouseEvent): Point {
    const r = svg.getBoundingClientRect();
    return [
      Math.max(0, Math.min(1, (event.clientX - r.left) / r.width)),
      Math.max(0, Math.min(1, 1 - (event.clientY - r.top) / r.height)),
    ];
  }
  function move(x: number, y: number) {
    const values = points();
    values[selected] = [
      selected === 0
        ? 0
        : selected === values.length - 1
          ? 1
          : Math.max(
              values[selected - 1][0] + 0.001,
              Math.min(values[selected + 1][0] - 0.001, x),
            ),
      Math.max(0, Math.min(1, y)),
    ];
    store(values);
    draw();
  }
  svg.onpointerdown = (event) => {
    if (blocked() || !get()) return;
    event.preventDefault();
    svg.focus();
    const p = coordinates(event),
      values = points();
    const nearest = values.findIndex(
      (v) => Math.hypot(v[0] - p[0], v[1] - p[1]) < 0.045,
    );
    begin();
    if (nearest >= 0) selected = nearest;
    else {
      if (values.length >= 32) {
        end();
        return;
      }
      const close = values.findIndex((v) => Math.abs(v[0] - p[0]) < 0.003);
      if (close >= 0) selected = close;
      else {
        values.push(p);
        values.sort((a, b) => a[0] - b[0]);
        selected = values.indexOf(p);
        store(values);
      }
    }
    dragging = true;
    svg.setPointerCapture(event.pointerId);
    draw();
  };
  svg.onpointermove = (event) => {
    if (dragging) move(...coordinates(event));
  };
  svg.onpointerup = svg.onpointercancel = () => {
    if (dragging) {
      dragging = false;
      end();
      draw();
    }
  };
  function remove() {
    const values = points();
    if (selected === 0 || selected === values.length - 1) return;
    values.splice(selected, 1);
    edit((s) => {
      s.point_curves ??= [[], [], [], []];
      s.point_curves[channel()] = values;
    }, t("カーブの点を削除"));
    draw();
  }
  svg.onkeydown = (event) => {
    if (blocked()) return;
    if (["Delete", "Backspace"].includes(event.key)) {
      event.preventDefault();
      event.stopPropagation();
      remove();
    } else if (event.key.startsWith("Arrow")) {
      event.preventDefault();
      event.stopPropagation();
      const p = points()[selected],
        step = (event.shiftKey ? 10 : 1) / 255;
      begin();
      move(
        p[0] +
          (event.key === "ArrowRight"
            ? step
            : event.key === "ArrowLeft"
              ? -step
              : 0),
        p[1] +
          (event.key === "ArrowUp"
            ? step
            : event.key === "ArrowDown"
              ? -step
              : 0),
      );
      end();
    }
  };
  $("curve-delete").onclick = remove;
  $("curve-reset").onclick = () => {
    edit((s) => {
      s.point_curves ??= [[], [], [], []];
      s.point_curves[channel()] = [
        [0, 0],
        [1, 1],
      ];
      s.curves[channel()] = [0, 0.25, 0.5, 0.75, 1];
    }, t("トーンカーブをリセット"));
    draw();
  };
  for (const id of ["curve-input", "curve-output"])
    $(id).onchange = () => {
      if (blocked()) return;
      begin();
      move(
        Number($("curve-input").value) / 255,
        Number($("curve-output").value) / 255,
      );
      end();
    };
  return { draw };
}

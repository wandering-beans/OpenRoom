import { createView } from './components/view';
import { DevelopToolsLayout } from './components/DevelopToolsLayout';
import { createElement, type ReactNode } from 'react';
import { workflow as nativeWorkflow } from './backend';
import type { Invoke } from './backend';
import type { Settings, SettingsPatch, SettingKey, Mask, Shape, Point, Crop, Edit, ErrorHandler, SavedRecipe, WorkflowRequest } from './models';
interface ToolOptions { invoke: Invoke; get(): { settings: Settings | null; defaults: Settings | null }; edit: Edit; interact(change: (settings: Settings) => void): void; finishGesture(): void; refresh(): void; error: ErrorHandler; selectedIds(): number[]; selectionChanged(): void; save(): Promise<boolean>; busy(value?: boolean): boolean; rendering(): boolean; status(text: string): void; saved(id: number): void }
interface Drag { start: Point; end: Point; original?: Crop; crop?: Crop; anchor?: Point; direction?: Point; ratio?: number; handle?: string; shape?: Shape }
import { element as $, context2d } from './dom';
import { t } from './i18n';
import { estimateStraighteningAngle } from "./straighten";
import { maskCoverage } from "./mask-preview";
// Interaction tools share the same recipe edits and undo/save path as the sliders.

const copy = <T,>(value: T) => structuredClone(value);
const clamp = (value: number) => Math.max(0, Math.min(1, value));
const cropCorners = [['nw', 0, 0], ['ne', 1, 0], ['sw', 0, 1], ['se', 1, 1]] as const;
export function createDevelopTools({
  invoke,
  get,
  edit,
  interact,
  finishGesture,
  refresh,
  error,
  selectedIds,
  selectionChanged,
  save,
  busy,
  rendering,
  status,
  saved,
}: ToolOptions) {
  const workflow = <R extends WorkflowRequest,>(request: R) => nativeWorkflow(request, invoke);
  let mode = "",
    maskIndex = -1,
    drag: Drag | null = null,
    reference: Settings | null = null;
  let ratioLocked = true;
  let angleAdjusting = false;
  const maskImages = new Map<string, string>();
  let layerSignature = '';
  function maskImage(mask: Pick<Mask, 'shapes' | 'invert'>, width: number, height: number, overlay = false) {
    const key = JSON.stringify([mask.shapes, mask.invert, width, height, overlay]);
    if (maskImages.has(key)) return maskImages.get(key)!;
    const coverage = maskCoverage(mask, width, height);
    if (!coverage) return '';
    const canvas = document.createElement('canvas');
    canvas.width = width; canvas.height = height;
    const context = context2d(canvas);
    const image = context.createImageData(width, height);
    for (let i = 0; i < coverage.length; i++) {
      const gray = Math.round(coverage[i] * 255);
      image.data.set(overlay ? [255, 65, 40, Math.round(gray * .45)] : [gray, gray, gray, 255], i * 4);
    }
    context.putImageData(image, 0, 0);
    const url = canvas.toDataURL();
    if (maskImages.size >= 64) maskImages.delete(maskImages.keys().next().value!);
    maskImages.set(key, url);
    return url;
  }
  let presets: SavedRecipe[] = [],
    snapshots: SavedRecipe[] = [],
    previous: Settings | null = null,
    currentId: number | null = null;
  const layoutHost = $('develop-tools-layout');
  createView(layoutHost).render(<DevelopToolsLayout />);
  const sections = [$('crop-tool-panel') as HTMLDetailsElement, $('mask-tool-panel') as HTMLDetailsElement];
  const maskPalette = $<HTMLFieldSetElement>('mask-palette');
  const layersView = createView($('mask-layers'));
  const toolbar = $('develop-toolstrip');
  createView(toolbar).render((['', 'crop', 'brush'] as const).map((tool, index) => <button key={tool} data-tool={tool} title={[t('補正'), t('切り抜きと角度補正'), t('マスク')][index]} aria-label={[t('補正'), t('切り抜きと角度補正'), t('マスク')][index]} onClick={() => {
    setMode(tool);
    if (tool === 'brush') $('develop-panel').querySelector('.develop-scroll')!.scrollTop = 0;
    else if (tool) sections[0].scrollIntoView({block:'nearest'});
  }}><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true">
    {tool === '' ? <><path d="M3 6h18M3 12h18M3 18h18" /><circle cx="8" cy="6" r="2" /><circle cx="16" cy="12" r="2" /><circle cx="9" cy="18" r="2" /></> : tool === 'crop' ? <path d="M7 3v14h14M3 7h14v14M4 20 20 4" /> : <><circle cx="12" cy="12" r="8" strokeDasharray="2 2" /><circle cx="12" cy="12" r="5" opacity=".4" /></>}
  </svg></button>));
  const groups: Record<string, SettingKey[]> = {
    basic: [
      "exposure",
      "temperature",
      "raw_temperature",
      "raw_tint",
      "raster_temperature",
      "raster_reference_temperature",
      "raster_tint",
      "tint",
      "contrast",
      "highlights",
      "shadows",
      "whites",
      "blacks",
      "texture",
      "clarity",
      "dehaze",
      "vibrance",
      "saturation",
      "monochrome",
    ],
    color: [
      "curves",
      "point_curves",
      "mixer",
      "grading",
      "grading_balance",
      "grading_blending",
    ],
    detail: [
      "sharpen",
      "sharpen_radius",
      "sharpen_detail",
      "sharpen_masking",
      "noise_luma",
      "noise_color",
    ],
    geometry: [
      "crop",
      "angle",
      "quarter_turns",
      "flip_h",
      "flip_v",
      "perspective_h",
      "perspective_v",
      "scale",
      "distortion",
      "aspect",
      "offset_x",
      "offset_y",
      "constrain_crop",
    ],
    effects: [
      "vignette",
      "vignette_midpoint",
      "vignette_feather",
      "grain",
      "grain_size",
      "grain_roughness",
    ],
    masks: ["masks"],
  };
  const labels = [
    t("基本補正"),
    t("色・カーブ"),
    t("ディテール"),
    t("切り抜き・変形"),
    t("効果"),
    t("部分補正"),
  ];
  createView($('copy-fields')).render(Object.keys(groups).map((key, index) => <label key={key}><input type="checkbox" defaultChecked data-group={key} />{labels[index]}</label>));
  function fields() {
    return [...$("copy-fields").querySelectorAll<HTMLInputElement>("input:checked")].flatMap(
      (input) => groups[input.dataset.group!],
    );
  }
  function merge(value: Settings, patch: SettingsPatch) {
    for (const [key, entry] of Object.entries(patch))
      if (key !== "version" && key in value) Object.assign(value, { [key]: copy(entry) });
  }
  function selectedPatch(value: Settings): SettingsPatch {
    return Object.fromEntries(fields().map((key) => [key, copy(value[key])]));
  }
  const svg = $<SVGSVGElement>('develop-overlay');
  const overlayView = createView(svg);
  function draw() {
    const state = get();
    const cropping = mode === 'crop' || mode === 'straighten';
    const masking = Boolean(mode && !cropping);
    maskPalette.hidden = !masking || !state?.settings;
    sections[0].open = cropping;
    sections[1].open = masking;
    for (const section of $('develop-controls').querySelectorAll<HTMLElement>(':scope > details'))
      if (section.tagName === 'DETAILS' && section !== sections[0] && section !== sections[1]) section.hidden = masking;
    for (const id of ['develop-reset', 'develop-previous', 'develop-copy', 'develop-paste']) $(id).hidden = masking;
    $('mask-radius-row').hidden = mode !== 'brush';
    $('mask-feather-row').hidden = mode === 'linear';
    sections[0].classList.toggle('crop-panel', cropping);
    $('photo-tool-heading').textContent = cropping ? t("切り抜きと角度補正") : mode ? $('photo-tool').selectedOptions[0].textContent : t("編集ツール");
    $('photo-tool-picker').hidden = cropping;
    $('crop-reset').hidden = !cropping;
    $('crop-options').hidden = !cropping;
    $('crop-straighten').setAttribute('aria-pressed', String(mode === 'straighten'));
    $('crop-lock').setAttribute('aria-pressed', String(ratioLocked));
    $('rotation-options').hidden = mode !== 'crop' && mode !== 'straighten';
    $('straighten-help').hidden = mode !== 'straighten';
    const overlay: ReactNode[] = [];
    svg.style.display = state?.settings && mode ? "block" : "none";
    if (!state?.settings || !mode) { overlayView.render(null); return; }
    const image = $("full-image"),
      rect = image.getBoundingClientRect();
    Object.assign(svg.style, {
      left: `${rect.left}px`,
      top: `${rect.top}px`,
      width: `${rect.width}px`,
      height: `${rect.height}px`,
    });
    svg.setAttribute("viewBox", "0 0 1 1");
    svg.setAttribute("preserveAspectRatio", "none");
    const shape = (tag: string, attrs: Record<string, string | number>) => {
      const properties = Object.fromEntries(Object.entries(attrs).map(([key, value]) => [key.startsWith('data-') ? key : key.replace(/-([a-z])/g, (_, letter: string) => letter.toUpperCase()), value]));
      if (typeof properties.style === 'string') {
        overlay.push(createElement(tag, {...properties, key: overlay.length, style: {cursor: properties.style.slice('cursor:'.length)}}));
      } else overlay.push(createElement(tag, {...properties, key: overlay.length}));
    };
    if (mode === "crop") {
      const ratio = currentCropRatio();
      if ($('crop-ratio').value !== '0' && Math.abs(ratio / presetCropRatio() - 1) > .001) {
        $('crop-ratio').value = 'custom';
      }
      for (const [id, portrait] of [['crop-landscape', false], ['crop-portrait', true]] as const) {
        $(id).setAttribute('aria-pressed', String(portrait ? ratio < 1 : ratio > 1));
        $(id).disabled = !Number.isFinite(ratio) || Math.abs(ratio - 1) < 1e-6;
      }
      const [x, y, w, h] = drag?.crop || state.settings.crop;
      shape("path", {
        d: `M0 0H1V1H0Z M${x} ${y}V${y + h}H${x + w}V${y}Z`,
        fill: "#0008",
        "fill-rule": "evenodd",
        stroke: "none",
      });
      shape("rect", {
        x,
        y,
        width: w,
        height: h,
        fill: "none",
        stroke: "white",
        "stroke-width": 0.003,
      });
      svg.dataset.cropOverlay = $('crop-overlay').value;
      for (const t of angleAdjusting ? [] : [1 / 3, 2 / 3])
        shape("path", {
          'data-crop-grid': '',
          d: `M${x + w * t} ${y}V${y + h} M${x} ${y + h * t}H${x + w}`,
          stroke: "#fff9",
          "stroke-width": 0.001,
        });
      for (const [name, right, bottom] of cropCorners) {
        const width = Math.min(w, 8 / rect.width), height = Math.min(h, 8 / rect.height);
        shape('rect', {
          'data-crop-corner': name,
          x: x + right * (w - width), y: y + bottom * (h - height), width, height,
          fill: 'white', style: `cursor:${right === bottom ? 'nwse' : 'nesw'}-resize`,
        });
      }
    } else {
      if (mode === 'straighten' && drag) shape('line', {
        x1:drag.start[0], y1:drag.start[1], x2:drag.end[0], y2:drag.end[1], stroke:'white', 'stroke-width':.003,
      });
      const shapes = [
        ...(state.settings.masks?.[maskIndex]?.shapes || []),
        ...(drag?.shape ? [drag.shape] : []),
      ];
      const mask = state.settings.masks?.[maskIndex];
      if (masking && $('mask-overlay').checked && shapes.length) {
        const scale = Math.min(1, 320 / Math.max(rect.width, rect.height));
        const preview = maskImage({shapes, invert: mask?.invert ?? false},
          Math.max(1, Math.round(rect.width * scale)), Math.max(1, Math.round(rect.height * scale)), true);
        if (preview) shape('image', { 'data-mask-overlay': '', href: preview, x: 0, y: 0, width: 1, height: 1, preserveAspectRatio: 'none', 'pointer-events': 'none' });
      }
      for (const s of shapes) {
        if (masking && !$('mask-overlay').checked && !drag) continue;
        const attrs = {
          fill: "none",
          stroke: s.operation === "subtract" ? "#4ff" : "#f55",
          "stroke-width": 0.003,
        };
        if (s.kind === "brush")
          shape("polyline", {
            ...attrs,
            points: s.points.map((p) => p.join(",")).join(" "),
            "stroke-width": 1,
            "vector-effect": "non-scaling-stroke",
            opacity: 0.65,
          });
        else if (s.kind === "radial")
          shape("ellipse", {
            ...attrs,
            cx: s.start[0],
            cy: s.start[1],
            rx: Math.abs(s.end[0] - s.start[0]),
            ry: Math.abs(s.end[1] - s.start[1]),
          });
        else if (s.kind === "linear")
          shape("line", {
            ...attrs,
            x1: s.start[0],
            y1: s.start[1],
            x2: s.end[0],
            y2: s.end[1],
          });
      }
    }
    if (cropping && angleAdjusting && rect.width > 0 && rect.height > 0) {
      const [x, y, w, h] = state.settings.crop;
      const lines = [];
      // Keep guides level and 32 screen pixels apart, including at 100% zoom.
      for (let offset = 32; offset < w * rect.width; offset += 32)
        lines.push(`M${x + offset / rect.width} ${y}v${h}`);
      for (let offset = 32; offset < h * rect.height; offset += 32)
        lines.push(`M${x} ${y + offset / rect.height}h${w}`);
      for (const [stroke, width] of [['#0008', 2], ['#fffb', 1]])
        shape('path', {
          'data-level-grid': '', d: lines.join(' '), fill: 'none', stroke,
          'stroke-width': width, 'vector-effect': 'non-scaling-stroke', 'pointer-events': 'none',
        });
    }
    overlayView.render(overlay);
  }
  const resize = new ResizeObserver(draw);
  resize.observe($("full-image"));
  $("image-area").addEventListener("scroll", draw);
  window.addEventListener("resize", draw);
  $("full-image").addEventListener("load", draw);
  function point(event: MouseEvent): Point {
    const r = svg.getBoundingClientRect();
    return [
      clamp((event.clientX - r.left) / r.width),
      clamp((event.clientY - r.top) / r.height),
    ];
  }
  function cropCornerAt(point: Point, crop: Crop) {
    const rect = svg.getBoundingClientRect();
    const [x, y, width, height] = crop;
    // Use screen pixels so the corner hit area stays usable at any zoom level.
    return cropCorners.map(([name, right, bottom]) => ({
      name, right, bottom,
      distance: Math.hypot((point[0] - x - right * width) * rect.width, (point[1] - y - bottom * height) * rect.height),
    })).filter(corner => corner.distance <= 14).sort((a, b) => a.distance - b.distance)[0];
  }
  function newMask(shape: Shape): Mask {
    return {
      name: t("マスク {0}", get().settings!.masks.length + 1),
      enabled: true,
      invert: false,
      amount: 1,
      shapes: [shape],
      adjustments: {},
    };
  }
  function canAddMaskContent(shapes: Shape[], newMaskCount = 0) {
    const masks = get().settings!.masks;
    const countPoints = (items: Shape[]) =>
      items.reduce((sum, shape) => sum + (shape.points?.length || 0), 0);
    const points = masks.reduce(
      (sum, mask) => sum + countPoints(mask.shapes),
      countPoints(shapes),
    );
    if (
      masks.length + newMaskCount > 32 ||
      points > 32768 ||
      (!newMaskCount && (masks[maskIndex]?.shapes.length || 0) + shapes.length > 256)
    ) {
      status(t("マスクの上限に達しました。不要なマスクや描画を削除してください。"));
      return false;
    }
    return true;
  }
  function colorAt(p: Point): [number, number, number] {
    const image = $("full-image"),
      canvas = document.createElement("canvas");
    canvas.width = image.naturalWidth;
    canvas.height = image.naturalHeight;
    const ctx = context2d(canvas);
    ctx.drawImage(image, 0, 0);
    return [
      ...ctx.getImageData(
        Math.min(canvas.width - 1, Math.floor(p[0] * canvas.width)),
        Math.min(canvas.height - 1, Math.floor(p[1] * canvas.height)),
        1,
        1,
      ).data,
    ]
      .slice(0, 3)
      .map((v) => v / 255) as [number, number, number];
  }
  svg.onpointerdown = (event) => {
    if (!get()?.settings || !mode || busy()) return;
    event.preventDefault();
    svg.setPointerCapture(event.pointerId);
    const start = point(event);
    drag = { start, end: start };
    if (mode === "crop") {
      const c = get().settings!.crop;
      drag.original = copy(c);
      drag.crop = copy(c);
      const corner = cropCornerAt(start, c);
      drag.ratio = selectedCropRatio();
      if (corner) {
        drag.anchor = [c[0] + (1 - corner.right) * c[2], c[1] + (1 - corner.bottom) * c[3]];
        drag.direction = [corner.right ? 1 : -1, corner.bottom ? 1 : -1];
      }
      drag.handle =
        corner
          ? "resize"
          : start[0] > c[0] &&
              start[0] < c[0] + c[2] &&
              start[1] > c[1] &&
              start[1] < c[1] + c[3] &&
              (c[2] < 0.99 || c[3] < 0.99)
            ? "move"
            : "new";
    } else if (mode !== "straighten") {
      drag.shape = {
        kind: mode,
        operation: 'add',
        start,
        end: start,
        radius: Number($("mask-radius").value),
        feather: Number($("mask-feather").value),
        opacity: 1,
        points: [start],
        color: [0.5, 0.5, 0.5],
        range: [0.25, 0.75],
      };
      if (mode === "color") {
        try {
          drag.shape.color = colorAt(start);
        } catch (failure) {
          drag = null;
          error(failure);
          return;
        }
      }
    }
    draw();
  };
  svg.onpointermove = (event) => {
    const p = point(event);
    if (!drag) {
      const corner = mode === 'crop' && get()?.settings && cropCornerAt(p, get().settings!.crop);
      svg.style.cursor = corner ? `${corner.right === corner.bottom ? 'nwse' : 'nesw'}-resize` : 'crosshair';
      return;
    }
    drag.end = p;
    if (mode === "crop") {
      const [x, y, w, h] = drag.original!;
      if (drag.handle === "move")
        drag.crop = [
          Math.max(0, Math.min(1 - w, x + p[0] - drag.start[0])),
          Math.max(0, Math.min(1 - h, y + p[1] - drag.start[1])),
          w,
          h,
        ];
      else {
        const anchor = drag.anchor || drag.start;
        const [directionX, directionY] = drag.direction || [p[0] < anchor[0] ? -1 : 1, p[1] < anchor[1] ? -1 : 1];
        const maxWidth = directionX > 0 ? 1 - anchor[0] : anchor[0];
        const maxHeight = directionY > 0 ? 1 - anchor[1] : anchor[1];
        let width = (p[0] - anchor[0]) * directionX;
        let height = (p[1] - anchor[1]) * directionY;
        const ratio = drag.ratio;
        const image = $('full-image');
        const aspect = Number(image.getAttribute('width')) / Number(image.getAttribute('height'));
        if (ratio) {
          const target = ratio / aspect;
          const minimum = Math.max(.01, .01 / target);
          const maximum = Math.min(maxHeight, maxWidth / target);
          if (maximum < minimum) return;
          const rect = svg.getBoundingClientRect();
          // Project the pointer onto the fixed-ratio diagonal in screen pixels.
          height = (width * target * rect.width ** 2 + height * rect.height ** 2) / ((target * rect.width) ** 2 + rect.height ** 2);
          height = Math.max(minimum, Math.min(maximum, height));
          width = height * target;
        } else {
          if (maxWidth < .01 || maxHeight < .01) return;
          width = Math.max(.01, Math.min(maxWidth, width));
          height = Math.max(.01, Math.min(maxHeight, height));
        }
        drag.crop = [anchor[0] - (directionX < 0 ? width : 0), anchor[1] - (directionY < 0 ? height : 0), width, height];
      }
    } else if (drag.shape) {
      drag.shape.end = p;
      if (mode === "brush" && drag.shape.points.length < 4096) {
        const last = drag.shape.points.at(-1)!;
        if (Math.hypot(p[0] - last[0], p[1] - last[1]) > 0.002)
          drag.shape.points.push(p);
      }
    }
    draw();
  };
  svg.onpointerup = (event) => {
    if (!drag) return;
    const value = drag;
    drag = null;
    svg.releasePointerCapture(event.pointerId);
    if (mode === "crop") {
      if (value.crop![2] >= 0.01 && value.crop![3] >= 0.01)
        edit((s) => {
          s.crop = value.crop!;
        }, t("切り抜き"));
    } else if (mode === "straighten") {
      const r = svg.getBoundingClientRect(),
        dx = (value.end[0] - value.start[0]) * r.width,
        dy = (value.end[1] - value.start[1]) * r.height;
      if (Math.hypot(dx, dy) > 5) {
        let angle = (Math.atan2(dy, dx) * 180) / Math.PI;
        angle = ((angle + 225) % 90) - 45;
        edit((s) => {
          s.angle = Math.max(-45, Math.min(45, s.angle - angle));
        }, t("水平線の補正"));
      }
      setMode("crop");
    } else if (
      value.shape &&
      canAddMaskContent([value.shape], get().settings!.masks[maskIndex] ? 0 : 1)
    )
      edit((s) => {
        if (maskIndex < 0 || !s.masks[maskIndex]) {
          s.masks.push(newMask(value.shape!));
          maskIndex = s.masks.length - 1;
        } else s.masks[maskIndex].shapes.push(value.shape!);
      }, t("マスクを描画"));
    draw();
  };
  svg.onpointerenter = () => svg.classList.add('crop-hover');
  svg.onpointerleave = () => svg.classList.remove('crop-hover');
  $('crop-overlay').onchange = draw;
  svg.onpointercancel = () => {
    drag = null;
    draw();
  };
  function previewSettings(settings: Settings) {
    const value = copy(settings);
    if (mode) {
      value.crop = [0, 0, 1, 1];
      value.constrain_crop = false;
    }
    return value;
  }
  function setMode(value: string) {
    finishGesture();
    const settings = get().settings;
    const previousPreview = settings && previewSettings(settings);
    const hadComparison = Boolean($('compare-mode').value);
    mode = value;
    angleAdjusting = false;
    sections[0].open = Boolean(value);
    sections[1].open = Boolean(value && value !== "crop" && value !== "straighten");
    $("crop-feedback").hidden = true;
    $("photo-tool").value = value;
    const selectedTool = !value ? '' : ['crop', 'straighten'].includes(value) ? 'crop' : 'brush';
    for (const button of toolbar.querySelectorAll('button'))
      button.setAttribute("aria-pressed", String(button.dataset.tool === selectedTool));
    reference = null;
    $("compare-mode").value = "";
    // Switching tools alone must not reopen a photo on a disconnected drive.
    if (settings && (hadComparison || JSON.stringify(previousPreview) !== JSON.stringify(previewSettings(settings)))) {
      refresh();
    }
    draw();
    selectionChanged();
  }
  $("photo-tool").onchange = (event) => setMode((event.target as HTMLInputElement).value);
  $("tool-done").onclick = () => setMode("");
  function currentCropRatio() {
    const image = $('full-image');
    const [, , width, height] = get().settings!.crop;
    return width * Number(image.getAttribute('width')) / (height * Number(image.getAttribute('height')));
  }
  function selectedCropRatio() {
    return ratioLocked ? currentCropRatio() : 0;
  }
  function presetCropRatio() {
    const image = $('full-image');
    const value = $('crop-ratio').value;
    if (value === 'original') return Number(image.getAttribute('width')) / Number(image.getAttribute('height'));
    if (value === 'custom') return currentCropRatio();
    const ratio = Number(value);
    return currentCropRatio() < 1 ? 1 / ratio : ratio;
  }
  function applyCropRatio(ratio: number) {
    if (!['crop', 'straighten'].includes(mode) || busy()) return;
    const image = $('full-image');
    const target = ratio * Number(image.getAttribute('height')) / Number(image.getAttribute('width'));
    if (!Number.isFinite(target) || target < .01 || target > 100) {
      status(t("この縦横比では、切り抜き範囲を写真内に収められません。"));
      return;
    }
    edit(settings => {
      const [x, y, width, height] = settings.crop;
      // Preserve area and center where the image bounds and minimum crop size allow it.
      const nextHeight = Math.min(1, 1 / target, Math.max(.01, .01 / target, Math.sqrt(width * height / target)));
      const nextWidth = nextHeight * target;
      settings.crop = [
        Math.max(0, Math.min(1 - nextWidth, x + width / 2 - nextWidth / 2)),
        Math.max(0, Math.min(1 - nextHeight, y + height / 2 - nextHeight / 2)),
        nextWidth, nextHeight,
      ];
    }, t("切り抜きの縦横比"));
  }
  $('crop-ratio').onchange = () => {
    ratioLocked = $('crop-ratio').value !== '0';
    const ratio = ratioLocked ? presetCropRatio() : 0;
    if (ratio) applyCropRatio(ratio);
    draw();
  };
  for (const [id, portrait] of [['crop-landscape', false], ['crop-portrait', true]] as const) {
    $(id).onclick = () => {
      if (portrait === (currentCropRatio() < 1)) return;
      const ratio = Math.max(selectedCropRatio() || currentCropRatio(), 1 / (selectedCropRatio() || currentCropRatio()));
      applyCropRatio(portrait ? 1 / ratio : ratio);
    };
  }
  $('crop-lock').onclick = () => {
    ratioLocked = !ratioLocked;
    if (ratioLocked && $('crop-ratio').value === '0') $('crop-ratio').value = 'custom';
    draw();
  };
  $('crop-orientation').onclick = () => {
    applyCropRatio(1 / currentCropRatio());
  };
  $('crop-straighten').onclick = () => setMode(mode === 'straighten' ? 'crop' : 'straighten');
  $('crop-reset').onclick = () => {
    ratioLocked = true; $('crop-ratio').value = 'original';
    edit(s => { s.crop = [0, 0, 1, 1]; s.angle = 0; s.constrain_crop = false; }, t("切り抜きと角度をリセット"));
    setMode('crop');
  };
  $('crop-auto').onclick = () => {
    const feedback = $('crop-feedback');
    const message = (text: string) => { feedback.textContent = text; feedback.hidden = false; };
    if (busy() || rendering()) { message(t("描画が完了してから自動補正を実行してください。")); return; }
    try {
      const image = $('full-image');
      if (!image.complete || !image.naturalWidth) return;
      const canvas = document.createElement('canvas');
      const scale = Math.min(1, 512 / Math.max(image.naturalWidth, image.naturalHeight));
      canvas.width = Math.round(image.naturalWidth * scale); canvas.height = Math.round(image.naturalHeight * scale);
      const context = canvas.getContext('2d', {willReadFrequently:true})!;
      context.drawImage(image, 0, 0, canvas.width, canvas.height);
      const angle = estimateStraighteningAngle(context.getImageData(0, 0, canvas.width, canvas.height));
      if (angle == null) { message(t("水平・垂直の基準を判定できません。定規で線を指定してください。")); return; }
      edit(s => { s.angle = Math.max(-45, Math.min(45, Math.round((s.angle - angle) * 100) / 100)); }, t("角度の自動補正"));
      message(t("写真の直線を基準に角度を補正しました。"));
    } catch (failure) { error(failure); }
  };
  $("crop-clear").onclick = () =>
    edit((s) => {
      s.crop = [0, 0, 1, 1];
    }, t("切り抜き解除"));
  $("constrain-crop").onchange = (event) =>
    edit((s) => {
      s.constrain_crop = (event.target as HTMLInputElement).checked;
    }, t("画像に固定"));
  $("mask-new").onclick = () => {
    $('mask-types').hidden = !$('mask-types').hidden;
    $('mask-new').setAttribute('aria-expanded', String(!$('mask-types').hidden));
  };
  for (const button of $('mask-types').querySelectorAll('button')) button.onclick = () => {
    finishGesture();
    maskIndex = -1;
    $('mask-types').hidden = true;
    $('mask-new').setAttribute('aria-expanded', 'false');
    setMode(button.dataset.maskType!);
  };
  $('mask-overlay').onchange = draw;
  for (const key of ['radius', 'feather'] as const) {
    const input = $(`mask-${key}`);
    input.oninput = () => {
      if (!get().settings!.masks[maskIndex]) return;
      interact(s => {
        for (const shape of s.masks[maskIndex].shapes)
          if (shape.kind === mode) shape[key] = Number(input.value);
      });
    };
    input.onchange = input.onblur = input.onpointercancel = finishGesture;
  }
  $('mask-done').onclick = () => setMode('');
  $('mask-reset').onclick = () => edit(s => {
    if (s.masks[maskIndex]) s.masks[maskIndex].adjustments = {};
  }, t("マスクの補正をリセット"));
  $('local-monochrome').onchange = event => edit(s => {
    if (s.masks[maskIndex]) s.masks[maskIndex].adjustments.monochrome = (event.target as HTMLInputElement).checked ? 1 : 0;
  }, t("マスクの白黒"));
  for (const [id, direction] of [['mask-up', 1], ['mask-down', -1]] as const) $(id).onclick = () => edit(s => {
    const next = maskIndex + direction;
    if (maskIndex < 0 || next < 0 || next >= s.masks.length) return;
    [s.masks[maskIndex], s.masks[next]] = [s.masks[next], s.masks[maskIndex]];
    maskIndex = next;
  }, t("マスクの順序を変更"));
  $("mask-copy").onclick = () => {
    const source = get().settings!.masks[maskIndex];
    if (!source || !canAddMaskContent(source.shapes, 1)) return;
    edit((s) => {
      if (s.masks[maskIndex]) {
        const mask = copy(s.masks[maskIndex]);
        mask.name = mask.name.slice(0, 70) + t(" コピー");
        s.masks.push(mask);
        maskIndex = s.masks.length - 1;
      }
    }, t("マスク複製"));
  };
  $("mask-delete").onclick = () =>
    edit((s) => {
      if (maskIndex >= 0) s.masks.splice(maskIndex, 1);
      maskIndex = -1;
    }, t("マスク削除"));
  $('mask-name').onchange = event => edit(value => {
    if (value.masks[maskIndex]) value.masks[maskIndex].name = (event.target as HTMLInputElement).value;
  }, t('マスク設定'));
  $("mask-amount").oninput = event => interact(s => {
    if (s.masks[maskIndex]) s.masks[maskIndex].amount = Number((event.target as HTMLInputElement).value);
  });
  $("mask-amount").onchange = $("mask-amount").onblur = $("mask-amount").onpointercancel = finishGesture;
  const optionViews = new Map<HTMLSelectElement, ReturnType<typeof createView>>();
  function options(select: HTMLSelectElement, rows: SavedRecipe[], empty: string) {
    const old = select.value;
    let view = optionViews.get(select);
    if (!view) { view = createView(select); optionViews.set(select, view); }
    view.render(<><option value="">{empty}</option>{rows.map(row => <option key={row.id} value={row.id}>{row.name}</option>)}</>);
    if (rows.some(row => String(row.id) === old)) select.value = old;
  }
  function sync() {
    const state = get();
    if (!state?.settings) return;
    const s = state.settings;
    if (maskIndex >= s.masks.length) maskIndex = -1;
    const photo = $('full-image');
    const aspect = Number(photo.getAttribute('width')) / Number(photo.getAttribute('height')) || 1;
    const thumbWidth = Math.max(1, Math.round(40 * Math.min(1, aspect)));
    const thumbHeight = Math.max(1, Math.round(40 * Math.min(1, 1 / aspect)));
    const signature = JSON.stringify([maskIndex, thumbWidth, thumbHeight,
      s.masks.map(mask => [mask.name, mask.enabled, mask.invert, mask.shapes])]);
    if (signature !== layerSignature) {
      layerSignature = signature;
      layersView.render(s.masks.map((mask, index) => <div key={index} className="mask-layer-row">
        <button className="mask-layer" data-mask-index={index} aria-pressed={index === maskIndex} onClick={() => {
          finishGesture(); maskIndex = index;
          setMode(['brush', 'linear', 'radial'].includes(mask.shapes[0]?.kind) ? mask.shapes[0].kind : 'brush');
        }}><img alt="" width={40} height={40} src={maskImage(mask, thumbWidth, thumbHeight) || undefined} /><span>{mask.name + (mask.enabled ? '' : t('（無効）'))}</span></button>
        {index === maskIndex && <div id="mask-layer-settings">
          <label><input id="mask-enabled" type="checkbox" checked={mask.enabled} onChange={event => edit(value => { value.masks[index].enabled = event.target.checked; }, t('マスク設定'))} />{t('有効')}</label>
          <label><input id="mask-invert" type="checkbox" checked={mask.invert} onChange={event => edit(value => { value.masks[index].invert = event.target.checked; }, t('マスク設定'))} />{t('反転')}</label>
        </div>}
      </div>).reverse());
    }
    const mask = s.masks[maskIndex];
    $('mask-draw-help').textContent = mask ? t("上のレイヤーほど後から適用します。写真をドラッグすると選択中のレイヤーに範囲を追加します。") : t("写真をドラッグして新しいマスクの範囲を指定してください。");
    $('local-heading').textContent = mask ? t("{0}の基本補正", mask.name) : t("マスクの基本補正");
    for (const id of ['mask-copy', 'mask-delete', 'mask-reset', 'local-monochrome'] as const) $(id).disabled = !mask;
    $('local-monochrome').checked = Boolean(mask?.adjustments.monochrome);
    $('mask-up').disabled = !mask || maskIndex === s.masks.length - 1;
    $('mask-down').disabled = !mask || maskIndex === 0;
    const shape = mask?.shapes.findLast(shape => shape.kind === mode);
    if (shape) for (const key of ['radius', 'feather'] as const) {
      const input = $(`mask-${key}`);
      if (document.activeElement !== input) input.value = String(shape[key]);
    }
    for (const key of ["name", "amount"] as const) {
      const input = $(`mask-${key}`);
      input.disabled = !mask;
      input.value = String(mask?.[key] ?? (key === "amount" ? 1 : ""));
    }
    $("constrain-crop").checked = s.constrain_crop;
    draw();
  }

  async function loadLists() {
    const id = currentId;
    try {
      const values = await Promise.all([
        workflow({ action: "list", item_id: null }),
        workflow({ action: "list", item_id: id }),
      ]);
      if (id !== currentId) return;
      [presets, snapshots] = values;
      options($("user-presets"), presets, t("プリセットを選択"));
      options($("snapshot-list"), snapshots, t("スナップショットを選択"));
    } catch (failure) {
      error(failure);
    }
  }
  function name() {
    const value = $("recipe-name").value.trim();
    if (!value) throw new Error(t("保存名を入力してください"));
    return value;
  }
  function action(id: string, handler: () => void | Promise<void>) {
    $(id).onclick = async () => {
      try {
        await handler();
      } catch (failure) {
        error(failure);
      }
    };
  }
  function row(kind: string) {
    return (kind === "preset" ? presets : snapshots).find(
      (row) =>
        String(row.id) ===
        $(kind === "preset" ? "user-presets" : "snapshot-list").value,
    );
  }
  for (const kind of ["preset", "snapshot"]) {
    action(`${kind}-save`, async () => {
      await workflow({
        action: "store",
        item_id: kind === "snapshot" ? currentId : null,
        name: name(),
        settings: get().settings!,
        fields: fields(),
      });
      await loadLists();
    });
    action(`${kind}-rename`, async () => {
      const value = row(kind);
      if (value) {
        await workflow({ action: "rename", id: value.id, name: name() });
        await loadLists();
      }
    });
    action(`${kind}-delete`, async () => {
      const value = row(kind);
      if (value) {
        await workflow({ action: "delete", id: value.id });
        await loadLists();
      }
    });
  }
  action("preset-apply", () => {
    const value = row("preset");
    if (value)
      edit(
        (s) => merge(s, JSON.parse(value.recipe)),
        t("プリセット：{0}", value.name),
      );
  });
  action("snapshot-restore", () => {
    const value = row("snapshot");
    if (value)
      edit(
        (s) => Object.assign(s, copy(get().defaults!), JSON.parse(value.recipe)),
        t("スナップショット：{0}", value.name),
      );
  });
  action("snapshot-compare", () => {
    const value = row("snapshot");
    if (value) {
      reference = { ...copy(get().defaults!), ...JSON.parse(value.recipe) };
      mode = "";
      $("photo-tool").value = "";
      $("compare-mode").value = "side";
      refresh();
      draw();
    }
  });
  action("preset-export", async () => {
    const value = row("preset");
    if (value)
      await invoke("preset_file", {
        write: true,
        value: JSON.parse(value.recipe),
      });
  });
  action("preset-import", async () => {
    const value = await invoke("preset_file", { write: false, value: null });
    if (value) {
      await workflow({
        action: "store",
        item_id: null,
        name: name(),
        settings: { ...copy(get().defaults!), ...value },
        fields: Object.keys(value).filter((key) => key !== "version"),
      });
      await loadLists();
    }
  });
  $("fields-modified").onclick = () => {
    const s = get();
    for (const input of $("copy-fields").querySelectorAll("input"))
      input.checked = groups[input.dataset.group!].some(
        (key) =>
          JSON.stringify(s.settings![key]) !== JSON.stringify(s.defaults![key]),
      );
  };
  async function synchronize(settings: Settings) {
    const ids = selectedIds().filter((id) => id !== currentId);
    if (!ids.length) return;
    await workflow({
      action: "apply",
      ids,
      settings: copy(settings),
      fields: fields(),
    });
    for (const id of ids) saved(id);
    status(t("{0}枚に設定を同期しました", ids.length));
  }
  action("develop-sync", async () => {
    if (!(await save())) return;
    busy(true);
    try {
      await synchronize(get().settings!);
    } finally {
      busy(false);
    }
  });
  $("develop-previous").onclick = () => {
    if (previous)
      edit((s) => merge(s, selectedPatch(previous!)), t("前の写真の設定"));
  };
  $("compare-mode").onchange = () => {
    mode = "";
    $("photo-tool").value = "";
    refresh();
    draw();
  };
  $("compare-current").onclick = () => {
    reference = copy(get().settings!);
    refresh();
  };
  $("compare-default").onclick = () => {
    reference = null;
    refresh();
  };
  return {
    finishCrop() {
      if (!get().settings || drag || !['crop', 'straighten'].includes(mode)) return false;
      setMode('');
      return true;
    },
    selectedMask() { return get()?.settings?.masks[maskIndex]; },
    sync,
    fields,
    selectedPatch,
    paste(patch: SettingsPatch) {
      edit((s) => merge(s, patch), t("設定を貼り付け"));
    },
    previewSettings,
    comparison() {
      return $("compare-mode").value
        ? {
            settings: copy(reference || get().defaults!),
            mode: $("compare-mode").value,
          }
        : null;
    },
    reference(settings: Settings) {
      reference = copy(settings);
      $("compare-mode").value = "side";
      setModeForReference();
    },
    async historyRename(id: number) {
      await workflow({
        action: "rename_history",
        item_id: currentId!,
        id,
        name: name(),
      });
    },
    async afterSave(settings: Settings) {
      if ($("auto-sync").checked) await synchronize(settings);
    },
    select(id: number | null | undefined, old: Settings | null) {
      if (old) previous = copy(old);
      currentId = id ?? null;
      ratioLocked = true; $('crop-ratio').value = 'original';
      maskIndex = -1;
      mode = "";
      reference = null;
      drag = null;
      angleAdjusting = false;
      $("photo-tool").value = "";
      $("compare-mode").value = "";
      $("auto-sync").checked = false;
      draw();
      if (id) loadLists();
    },
    showLevelGrid(value: boolean) {
      if (angleAdjusting === value) return;
      angleAdjusting = value;
      draw();
    },
    update() {
      draw();
    },
  };
  function setModeForReference() {
    mode = "";
    $("photo-tool").value = "";
    refresh();
    draw();
  }
}

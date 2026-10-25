import type { Mask } from './models';
// Display-only coverage; development and export use masks.rs on the original pixels.
export function maskCoverage(mask: Pick<Mask, 'shapes'> & Partial<Pick<Mask, 'invert'>>, width: number, height: number) {
  if (mask.shapes.some(shape => !['brush', 'linear', 'radial'].includes(shape.kind))) return null;
  const clamp = (value: number) => Math.max(0, Math.min(1, value));
  const aspect = width / height;
  const combined = new Float32Array(width * height);
  for (const [index, shape] of mask.shapes.entries()) {
    const weights = new Float32Array(combined.length);
    const fade = (distance: number) => clamp((1 - distance) / Math.max(.001, shape.feather));
    if (shape.kind === 'brush') {
      for (const [point, start] of shape.points.entries()) {
        const end = shape.points[point + 1] || start;
        const radiusX = shape.radius / aspect;
        const left = Math.max(0, Math.floor((Math.min(start[0], end[0]) - radiusX) * width));
        const right = Math.min(width, Math.ceil((Math.max(start[0], end[0]) + radiusX) * width));
        const top = Math.max(0, Math.floor((Math.min(start[1], end[1]) - shape.radius) * height));
        const bottom = Math.min(height, Math.ceil((Math.max(start[1], end[1]) + shape.radius) * height));
        const dx = (end[0] - start[0]) * aspect, dy = end[1] - start[1];
        for (let y = top; y < bottom; y++) for (let x = left; x < right; x++) {
          const px = ((x + .5) / width - start[0]) * aspect, py = (y + .5) / height - start[1];
          const t = clamp((px * dx + py * dy) / Math.max(1e-12, dx * dx + dy * dy));
          const slot = y * width + x;
          weights[slot] = Math.max(weights[slot], fade(Math.hypot(px - t * dx, py - t * dy) / shape.radius));
        }
      }
    } else {
      const [sx, sy] = shape.start, [ex, ey] = shape.end;
      const dx = (ex - sx) * aspect, dy = ey - sy;
      const length = dx * dx + dy * dy;
      for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
        const nx = (x + .5) / width, ny = (y + .5) / height;
        weights[y * width + x] = shape.kind === 'linear'
          ? length < 1e-10 ? 0 : clamp(1 - ((nx - sx) * aspect * dx + (ny - sy) * dy) / length)
          : fade(Math.hypot((nx - sx) / Math.max(.001, Math.abs(ex - sx)), (ny - sy) / Math.max(.001, Math.abs(ey - sy))));
      }
    }
    for (let i = 0; i < combined.length; i++) {
      const selection = weights[i] * shape.opacity;
      combined[i] = shape.operation === 'subtract' ? combined[i] * (1 - selection)
        : shape.operation === 'intersect' && index > 0 ? combined[i] * selection : Math.max(combined[i], selection);
    }
  }
  if (mask.invert) for (let i = 0; i < combined.length; i++) combined[i] = 1 - combined[i];
  return combined;
}

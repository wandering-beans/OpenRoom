// Estimate a small rotation from long horizontal/vertical edges in a display-sized image.
export function estimateStraighteningAngle({data, width, height}: Pick<ImageData, 'data' | 'width' | 'height'>) {
  if (width < 32 || height < 32) return null;
  const gray = new Float32Array(width * height);
  for (let i = 0; i < gray.length; i++) {
    gray[i] = (data[i * 4] * .2126 + data[i * 4 + 1] * .7152 + data[i * 4 + 2] * .0722) * data[i * 4 + 3] / 255;
  }
  const smooth = new Float32Array(gray.length);
  for (let y = 1; y < height - 1; y++) for (let x = 1; x < width - 1; x++) {
    const i = y * width + x;
    smooth[i] = (gray[i - width - 1] + 2 * gray[i - width] + gray[i - width + 1]
      + 2 * gray[i - 1] + 4 * gray[i] + 2 * gray[i + 1]
      + gray[i + width - 1] + 2 * gray[i + width] + gray[i + width + 1]) / 16;
  }
  const edges = [];
  for (let y = 2; y < height - 2; y += 2) {
    for (let x = 2; x < width - 2; x += 2) {
      const i = y * width + x;
      // Rotated image boundaries are not scene horizons.
      if ([-2 * width, 2 * width, -2, 2].some(offset => data[(i + offset) * 4 + 3] < 250)) continue;
      const dx = smooth[i + 1] - smooth[i - 1], dy = smooth[i + width] - smooth[i - width];
      const weight = Math.hypot(dx, dy);
      if (weight < 40 || data[i * 4 + 3] < 250) continue;
      const vertical = Math.abs(dx) > Math.abs(dy);
      let angle = Math.atan2(vertical ? dy : -dx, vertical ? dx : dy) * 180 / Math.PI;
      angle = ((angle + 270) % 180) - 90;
      if (Math.abs(angle) > 20) continue;
      edges.push({x, y, weight: Math.min(weight, 100), vertical, angle});
    }
  }
  if (edges.length < 40) return null;
  const radius = Math.ceil(Math.hypot(width, height));
  const scores = [];
  for (let angle = -15; angle <= 15; angle += .25) {
    const radians = angle * Math.PI / 180, cosine = Math.cos(radians), sine = Math.sin(radians);
    const horizontal = new Float32Array(radius * 2 + 1), vertical = new Float32Array(radius * 2 + 1);
    for (const edge of edges) {
      if (Math.abs(edge.angle - angle) > 4) continue;
      const distance = edge.vertical ? edge.x * cosine + edge.y * sine : edge.y * cosine - edge.x * sine;
      const bins = edge.vertical ? vertical : horizontal;
      bins[Math.round(distance) + radius] += edge.weight;
    }
    let score = 0;
    for (let i = 1; i < horizontal.length - 1; i++) {
      score = Math.max(score, (horizontal[i - 1] + horizontal[i] + horizontal[i + 1]) / width,
        (vertical[i - 1] + vertical[i] + vertical[i + 1]) / height);
    }
    scores.push({angle, score});
  }
  scores.sort((a, b) => b.score - a.score || Math.abs(a.angle) - Math.abs(b.angle));
  const best = scores[0], alternative = scores.find(value => Math.abs(value.angle - best.angle) > 3);
  // shortcut: Conservative straight-edge detection; ambiguous scenes need the manual level tool.
  if (best.score < 18 || best.score < (alternative?.score ?? 0) * 1.15 || Math.abs(best.angle) >= 15) return null;
  return best.angle;
}

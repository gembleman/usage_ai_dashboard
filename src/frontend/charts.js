function renderTrendChart(rows) {
  const container = document.getElementById('trendChart');
  const legend = document.getElementById('trendLegend');
  container.innerHTML = '';
  legend.innerHTML = '';
  if (rows.length === 0) {
    container.innerHTML = '<div class="empty-note">데이터가 없습니다.</div>';
    return;
  }

  // date -> series -> total_tokens
  const dates = [...new Set(rows.map(r => r.date))].sort();
  const keys = [...new Set(rows.map(seriesKey))].sort();
  const byDateSeries = {};
  for (const d of dates) byDateSeries[d] = {};
  for (const r of rows) {
    const k = seriesKey(r);
    byDateSeries[r.date][k] = (byDateSeries[r.date][k] || 0) + r.total_tokens;
  }

  const width = Math.max(600, dates.length * 60);
  const height = 260;
  const padL = 56, padR = 16, padT = 12, padB = 28;
  const plotW = width - padL - padR;
  const plotH = height - padT - padB;

  const maxVal = Math.max(1, ...dates.map(d =>
    keys.reduce((sum, k) => sum + (byDateSeries[d][k] || 0), 0)
  ));

  const xStep = dates.length > 1 ? plotW / (dates.length - 1) : 0;
  const x = i => padL + (dates.length > 1 ? i * xStep : plotW / 2);
  const yFor = v => padT + plotH - (v / maxVal) * plotH;

  let svg = `<svg viewBox="0 0 ${width} ${height}" width="100%" style="max-width:100%;height:auto;display:block">`;

  // gridlines + y labels
  const gridSteps = 4;
  for (let i = 0; i <= gridSteps; i++) {
    const v = (maxVal / gridSteps) * i;
    const y = yFor(v);
    svg += `<line x1="${padL}" y1="${y}" x2="${width - padR}" y2="${y}" stroke="var(--grid-line)" stroke-width="1"/>`;
    svg += `<text x="${padL - 8}" y="${y + 3}" text-anchor="end">${fmtKo(v)}</text>`;
  }

  // x labels (thin out if too many)
  const labelEvery = Math.max(1, Math.ceil(dates.length / 12));
  dates.forEach((d, i) => {
    if (i % labelEvery === 0) {
      svg += `<text x="${x(i)}" y="${height - 8}" text-anchor="middle">${d.slice(5)}</text>`;
    }
  });

  // one stacked-per-series line each (not stacked, overlaid) — simpler & clearer for comparison
  keys.forEach(k => {
    const color = colorFor(k, keys);
    const points = dates.map((d, i) => [x(i), yFor(byDateSeries[d][k] || 0)]);
    const path = points.map((p, i) => (i === 0 ? 'M' : 'L') + p[0].toFixed(1) + ',' + p[1].toFixed(1)).join(' ');
    svg += `<path d="${path}" fill="none" stroke="${color}" stroke-width="2"/>`;
    points.forEach(([px, py], i) => {
      const val = byDateSeries[dates[i]][k] || 0;
      svg += `<circle cx="${px}" cy="${py}" r="3" fill="${color}" data-date="${dates[i]}" data-key="${k}" data-val="${val}" class="pt"/>`;
    });
  });

  svg += `</svg>`;
  container.innerHTML = svg;

  container.querySelectorAll('circle.pt').forEach(c => {
    c.addEventListener('mouseenter', evt => {
      const d = c.getAttribute('data-date');
      const k = c.getAttribute('data-key');
      const v = c.getAttribute('data-val');
      showTooltip(evt, `<b>${k}</b><br>${d}<br>${fmtKo(Number(v))} 토큰`);
    });
    c.addEventListener('mousemove', evt => showTooltip(evt, tooltip.innerHTML));
    c.addEventListener('mouseleave', hideTooltip);
  });

  keys.forEach(k => {
    const color = colorFor(k, keys);
    const item = document.createElement('div');
    item.className = 'legend-item';
    item.innerHTML = `<span class="swatch" style="background:${color}"></span>${k}`;
    legend.appendChild(item);
  });
}

function renderModelChart(rows) {
  const container = document.getElementById('modelChart');
  const legend = document.getElementById('modelLegend');
  container.innerHTML = '';
  legend.innerHTML = '';
  if (rows.length === 0) {
    container.innerHTML = '<div class="empty-note">데이터가 없습니다.</div>';
    return;
  }

  const byModel = {};
  const byModelUsage = {};
  for (const r of rows) {
    byModel[r.model] = (byModel[r.model] || 0) + r.total_tokens;
    const u = byModelUsage[r.model] || { input: 0, cached: 0, output: 0 };
    u.input += r.input_tokens;
    u.cached += r.cached_input_tokens;
    u.output += r.output_tokens;
    byModelUsage[r.model] = u;
  }
  const entries = Object.entries(byModel).sort((a, b) => b[1] - a[1]);
  const total = entries.reduce((s, [, v]) => s + v, 0) || 1;
  const totalCost = entries.reduce((s, [m]) => {
    const u = byModelUsage[m];
    const c = estimateCostUsd(m, u.input, u.cached, u.output);
    return s + (c || 0);
  }, 0);

  const size = 180, cx = size / 2, cy = size / 2, r = 78;
  let angle = -Math.PI / 2;
  let svg = `<svg viewBox="0 0 ${size + 140} ${size}" width="100%" style="max-width:420px;height:auto;display:block">`;
  entries.forEach(([model, val], i) => {
    const frac = val / total;
    const nextAngle = angle + frac * Math.PI * 2;
    const x1 = cx + r * Math.cos(angle), y1 = cy + r * Math.sin(angle);
    const x2 = cx + r * Math.cos(nextAngle), y2 = cy + r * Math.sin(nextAngle);
    const largeArc = frac > 0.5 ? 1 : 0;
    const color = PALETTE[i % PALETTE.length];
    const path = frac >= 0.9999
      ? `M ${cx} ${cy - r} A ${r} ${r} 0 1 1 ${cx - 0.01} ${cy - r} Z`
      : `M ${cx} ${cy} L ${x1} ${y1} A ${r} ${r} 0 ${largeArc} 1 ${x2} ${y2} Z`;
    svg += `<path d="${path}" fill="${color}" stroke="var(--panel)" stroke-width="1.5" class="slice" data-model="${model}" data-val="${val}" data-pct="${(frac*100).toFixed(1)}"/>`;
    angle = nextAngle;
  });
  svg += `</svg>`;
  container.innerHTML = svg;

  container.querySelectorAll('.slice').forEach(s => {
    s.addEventListener('mouseenter', evt => {
      const m = s.getAttribute('data-model');
      const v = s.getAttribute('data-val');
      const p = s.getAttribute('data-pct');
      const u = byModelUsage[m];
      const cost = estimateCostUsd(m, u.input, u.cached, u.output);
      showTooltip(evt, `<b>${m}</b><br>${fmtKo(Number(v))} 토큰 (${p}%)<br>예상 비용: ${fmtUsd(cost)}`);
    });
    s.addEventListener('mousemove', evt => showTooltip(evt, tooltip.innerHTML));
    s.addEventListener('mouseleave', hideTooltip);
  });

  const totalItem = document.createElement('div');
  totalItem.className = 'legend-item legend-total';
  totalItem.innerHTML = `<b>총 예상 비용: ${fmtUsd(totalCost)}</b>`;
  legend.appendChild(totalItem);

  entries.forEach(([model, val], i) => {
    const color = PALETTE[i % PALETTE.length];
    const pct = ((val / total) * 100).toFixed(1);
    const u = byModelUsage[model];
    const cost = estimateCostUsd(model, u.input, u.cached, u.output);
    const item = document.createElement('div');
    item.className = 'legend-item';
    item.innerHTML = `<span class="swatch" style="background:${color}"></span>${model} (${pct}%) — ${fmtUsd(cost)}`;
    legend.appendChild(item);
  });
}

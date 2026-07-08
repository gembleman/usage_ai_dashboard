// 현재 렌더링된 트렌드 차트의 데이터. 포인트 툴팁이 이벤트 위임 핸들러에서
// 참조한다 (포인트가 수천 개일 수 있어 각 원에 리스너를 붙이지 않는다).
let trendChartState = null;

function trendPointFromEvent(evt) {
  if (!trendChartState || !evt.target || !evt.target.closest) return null;
  return evt.target.closest('circle.pt');
}

function showTrendPointTooltip(evt, c) {
  const { dates, keys, byDateSeries } = trendChartState;
  const d = dates[Number(c.getAttribute('data-date-idx'))];
  const k = keys[Number(c.getAttribute('data-key-idx'))];
  showTooltip(evt, `<b>${escapeHtml(k)}</b><br>${escapeHtml(d)}<br>${fmtKo(byDateSeries[d][k])} 토큰`);
}

// 리스너는 컨테이너에 한 번만 위임한다. 렌더링마다 innerHTML을 갈아끼워도
// 컨테이너 자체에 붙은 리스너는 유지된다. (스크립트는 defer라 DOM 준비됨)
{
  const container = document.getElementById('trendChart');
  const onPoint = evt => {
    const c = trendPointFromEvent(evt);
    if (c) showTrendPointTooltip(evt, c);
  };
  const offPoint = evt => {
    if (trendPointFromEvent(evt)) hideTooltip();
  };
  container.addEventListener('mouseover', onPoint);
  container.addEventListener('mousemove', onPoint);
  container.addEventListener('mouseout', offPoint);
  container.addEventListener('focusin', onPoint);
  container.addEventListener('focusout', offPoint);
  container.addEventListener('touchstart', evt => {
    const c = trendPointFromEvent(evt);
    if (c) showTrendPointTooltip(evt.touches[0] || evt, c);
  }, { passive: true });
  container.addEventListener('touchend', offPoint);
}

function renderTrendChart(rows) {
  const container = document.getElementById('trendChart');
  const legend = document.getElementById('trendLegend');
  container.innerHTML = '';
  legend.innerHTML = '';
  if (rows.length === 0) {
    trendChartState = null;
    container.innerHTML = '<div class="empty-note">데이터가 없습니다.</div>';
    return;
  }

  // date -> series -> total_tokens (기록이 없는 조합은 undefined로 남겨 "데이터 없음"과 "0"을 구분한다)
  const dates = [...new Set(rows.map(r => r.date))].sort();
  const keys = [...new Set(rows.map(seriesKey))].sort();
  const byDateSeries = {};
  for (const d of dates) byDateSeries[d] = {};
  for (const r of rows) {
    const k = seriesKey(r);
    byDateSeries[r.date][k] = (byDateSeries[r.date][k] || 0) + r.total_tokens;
  }
  trendChartState = { dates, keys, byDateSeries };

  // 날짜가 많을수록 포인트 간격을 좁혀 SVG 전체 폭을 억제한다
  // (365일 × 60px ≈ 22,000px짜리 SVG는 렌더링/스크롤이 무겁다).
  const pxPerDate = dates.length <= 31 ? 60 : dates.length <= 120 ? 24 : 10;
  const pointRadius = dates.length <= 120 ? 3 : 2;
  const width = Math.max(600, dates.length * pxPerDate);
  const height = 260;
  const padL = 56, padR = 16, padT = 12, padB = 28;
  const plotW = width - padL - padR;
  const plotH = height - padT - padB;

  // 시리즈는 겹쳐서(overlay) 그리므로 Y축 최댓값은 "개별 시리즈 값의 최댓값"이어야 한다.
  // (합계로 계산하면 스택 차트용 스케일이 되어 모든 선이 하단에 압축되어 보인다.)
  const maxVal = Math.max(1, ...dates.flatMap(d =>
    keys.map(k => byDateSeries[d][k] || 0)
  ));

  const xStep = dates.length > 1 ? plotW / (dates.length - 1) : 0;
  const x = i => padL + (dates.length > 1 ? i * xStep : plotW / 2);
  const yFor = v => padT + plotH - (v / maxVal) * plotH;

  const chartTitle = `일별 토큰 사용량 추이 (${keys.length}개 시리즈, ${dates.length}일)`;
  let svg = `<svg viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" style="display:block" role="img" aria-label="${escapeHtml(chartTitle)}"><title>${escapeHtml(chartTitle)}</title>`;

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
      svg += `<text x="${x(i)}" y="${height - 8}" text-anchor="middle">${escapeHtml(d.slice(5))}</text>`;
    }
  });

  // 시리즈별로 한 줄씩 겹쳐 그린다(overlay, not stacked) — 시리즈 간 비교가 목적.
  // 데이터가 없는 날짜는 선을 끊어 "사용 안 함(0)"과 "데이터 없음"을 구분한다.
  keys.forEach((k, keyIdx) => {
    const color = colorFor(k, keys);
    let path = '';
    let started = false;
    dates.forEach((d, i) => {
      const raw = byDateSeries[d][k];
      if (raw === undefined) {
        started = false; // 데이터 없는 지점에서 선을 끊는다.
        return;
      }
      const px = x(i).toFixed(1);
      const py = yFor(raw).toFixed(1);
      path += (started ? 'L' : 'M') + px + ',' + py + ' ';
      started = true;
    });
    svg += `<path d="${path.trim()}" fill="none" stroke="${color}" stroke-width="2"/>`;
    dates.forEach((d, i) => {
      const raw = byDateSeries[d][k];
      if (raw === undefined) return; // 데이터 없는 날짜는 포인트를 생략한다.
      const px = x(i), py = yFor(raw);
      const label = `${k} — ${d}: ${fmtKo(raw)} 토큰`;
      svg += `<circle cx="${px}" cy="${py}" r="${pointRadius}" fill="${color}" tabindex="0" ` +
        `data-date-idx="${i}" data-key-idx="${keyIdx}" class="pt" aria-label="${escapeHtml(label)}"><title>${escapeHtml(label)}</title></circle>`;
    });
  });

  svg += `</svg>`;
  container.innerHTML = svg;

  keys.forEach(k => {
    const color = colorFor(k, keys);
    const item = document.createElement('div');
    item.className = 'legend-item';
    item.innerHTML = `<span class="swatch" style="background:${color}"></span>${escapeHtml(k)}`;
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
  const rawEntries = Object.entries(byModel).sort((a, b) => b[1] - a[1]);
  const total = rawEntries.reduce((s, [, v]) => s + v, 0) || 1;
  const totalCost = rawEntries.reduce((s, [m]) => {
    const u = byModelUsage[m];
    const c = estimateCostUsd(m, u.input, u.cached, u.output);
    return s + (c || 0);
  }, 0);

  // 1% 미만 조각은 식별이 어려우므로 "기타"로 묶는다.
  const OTHER_LABEL = '기타';
  const main = [];
  let otherVal = 0;
  // "기타"는 여러 실모델의 묶음이라 findPricing('기타')가 실패한다. 개별 모델 비용을
  // 여기서 미리 합산해 두고, 툴팁/범례에서 실시간 estimateCostUsd 대신 이 값을 쓴다.
  const otherUsage = { input: 0, cached: 0, output: 0, cost: 0, hasCost: false };
  for (const [model, val] of rawEntries) {
    if (val / total < 0.01) {
      otherVal += val;
      const u = byModelUsage[model];
      otherUsage.input += u.input;
      otherUsage.cached += u.cached;
      otherUsage.output += u.output;
      const c = estimateCostUsd(model, u.input, u.cached, u.output);
      if (c != null) { otherUsage.cost += c; otherUsage.hasCost = true; }
    } else {
      main.push([model, val]);
    }
  }
  const entries = otherVal > 0 ? [...main, [OTHER_LABEL, otherVal]] : main;
  if (otherVal > 0) byModelUsage[OTHER_LABEL] = otherUsage;

  const size = 180, cx = size / 2, cy = size / 2, r = 78;
  let angle = -Math.PI / 2;
  const chartTitle = `모델별 토큰 분포 (${entries.length}개 항목)`;
  let svg = `<svg viewBox="0 0 ${size} ${size}" width="${size}" height="${size}" style="max-width:100%;height:auto;display:block" role="img" aria-label="${escapeHtml(chartTitle)}"><title>${escapeHtml(chartTitle)}</title>`;
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
    const pct = (frac * 100).toFixed(1);
    const label = `${model}: ${fmtKo(val)} 토큰 (${pct}%)`;
    svg += `<path d="${path}" fill="${color}" stroke="var(--panel)" stroke-width="1.5" class="slice" ` +
      `tabindex="0" data-entry-idx="${i}" aria-label="${escapeHtml(label)}"><title>${escapeHtml(label)}</title></path>`;
    angle = nextAngle;
  });
  svg += `</svg>`;
  container.innerHTML = svg;

  // "기타"는 findPricing 매칭이 안 되므로 위에서 미리 합산한 비용(otherUsage)을 쓰고,
  // 실모델은 그대로 실시간 계산한다. 비용이 하나도 없으면 null → fmtUsd가 '—'로 표시한다.
  const costForEntry = model => {
    if (model === OTHER_LABEL) return otherUsage.hasCost ? otherUsage.cost : null;
    const u = byModelUsage[model];
    return estimateCostUsd(model, u.input, u.cached, u.output);
  };

  const showSliceTooltip = (evt, s) => {
    const i = Number(s.getAttribute('data-entry-idx'));
    const [model, val] = entries[i];
    const pct = ((val / total) * 100).toFixed(1);
    const cost = costForEntry(model);
    showTooltip(evt, `<b>${escapeHtml(model)}</b><br>${fmtKo(val)} 토큰 (${pct}%)<br>예상 비용: ${fmtUsd(cost)}`);
  };

  container.querySelectorAll('.slice').forEach(s => {
    s.addEventListener('mouseenter', evt => showSliceTooltip(evt, s));
    s.addEventListener('mousemove', evt => showSliceTooltip(evt, s));
    s.addEventListener('mouseleave', hideTooltip);
    s.addEventListener('focus', evt => showSliceTooltip(evt, s));
    s.addEventListener('blur', hideTooltip);
    s.addEventListener('touchstart', evt => { showSliceTooltip(evt.touches[0] || evt, s); }, { passive: true });
    s.addEventListener('touchend', hideTooltip);
  });

  const totalItem = document.createElement('div');
  totalItem.className = 'legend-item legend-total';
  totalItem.innerHTML = `<b>총 예상 비용: ${fmtUsd(totalCost)}</b>`;
  legend.appendChild(totalItem);

  entries.forEach(([model, val], i) => {
    const color = PALETTE[i % PALETTE.length];
    const pct = ((val / total) * 100).toFixed(1);
    const cost = costForEntry(model);
    const item = document.createElement('div');
    item.className = 'legend-item';
    item.innerHTML = `<span class="swatch" style="background:${color}"></span>${escapeHtml(model)} (${pct}%) — ${fmtUsd(cost)}`;
    legend.appendChild(item);
  });
}

import {
  chartColor,
  emptyNote,
  escapeHtml,
  fmtKo,
  fmtUsdPartial,
  hideTooltip,
  rowCostUsd,
  seriesKey,
  showTooltip,
  swatch,
} from './util.js';

// 현재 렌더링된 트렌드 차트의 데이터. 포인트 툴팁이 이벤트 위임 핸들러에서
// 참조한다 (포인트가 수천 개일 수 있어 각 원에 리스너를 붙이지 않는다).
let trendChartState = null;
let hourlyChartState = null;
let modelChartState = null;
let modelChartMaxItems = 8;
let seriesColorIndices = new Map();
let modelColorIndices = new Map();
const LINE_PATTERNS = [
  { dash: '', borderStyle: 'solid' },
  { dash: '8 4', borderStyle: 'dashed' },
  { dash: '2 4', borderStyle: 'dotted' },
];

// Assign colors from the complete data set so range filters do not change an
// account's or model's color. The daily and hourly charts share one domain.
export function setChartColorDomains(usageRows, hourlyRows) {
  const series = [...new Set([...usageRows, ...hourlyRows].map(seriesKey))].sort();
  seriesColorIndices = new Map(series.map((key, index) => [key, index]));

  const modelTotals = new Map();
  for (const row of usageRows) {
    modelTotals.set(row.model, (modelTotals.get(row.model) || 0) + row.total_tokens);
  }
  const models = [...modelTotals].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  modelColorIndices = new Map(models.map(([model], index) => [model, index]));
}

const seriesColor = (key, fallbackIndex) => chartColor(seriesColorIndices.get(key) ?? fallbackIndex);
const modelColor = (model, fallbackIndex) => model === '기타'
  ? '#c4cbd6'
  : chartColor(modelColorIndices.get(model) ?? fallbackIndex);

export function setModelChartMaxItems(value) {
  if (Number.isInteger(value) && value >= 2) modelChartMaxItems = value;
}

// 모델 단위로 합산된 비용. 각 행에서 실제 청구액 또는 가격표 추정액을 선택한 뒤
// 합산하므로, 같은 모델을 Codex와 pi/OpenCode가 함께 사용해도 어느 한쪽이 누락되지 않는다.
function modelCostUsd(u) {
  if (!u) return null;
  return u.hasCost ? u.cost : null;
}

// SVG y축 라벨이 길어질 때 필요한 왼쪽 여백을 실제 사용 글꼴로 계산한다.
// 기본 여백은 유지하고, 긴 숫자에 필요한 만큼만 차트 전체 폭을 늘린다.
function yAxisPadding(labels, minimum = 56) {
  const context = document.createElement('canvas').getContext('2d');
  if (!context) return minimum;
  const mono = getComputedStyle(document.documentElement).getPropertyValue('--mono').trim() || 'monospace';
  context.font = `10px ${mono}`;
  const widest = Math.max(0, ...labels.map(label => context.measureText(label).width));
  return Math.max(minimum, Math.ceil(widest) + 12);
}

// 차트 최댓값의 가장 큰 단위로 모든 y축 눈금을 통일한다.
// 예: 최댓값이 1,715,596,790이면 "17.16억"으로 간결하게 표시한다.
function compactYAxisLabels(maxValue, steps) {
  const units = [
    { minimum: 100_000_000, divisor: 100_000_000, suffix: '억' },
    { minimum: 10_000, divisor: 10_000, suffix: '만' },
    { minimum: 1_000, divisor: 1_000, suffix: '천' },
  ];
  const unit = units.find(candidate => maxValue >= candidate.minimum);
  const format = new Intl.NumberFormat('ko-KR', { maximumFractionDigits: 2 });
  return Array.from({ length: steps + 1 }, (_, i) => {
    const value = (maxValue / steps) * i;
    return unit ? `${format.format(value / unit.divisor)}${unit.suffix}` : format.format(value);
  });
}

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

// 리스너는 컨테이너에 한 번만 위임한다. 렌더링마다 SVG를 갈아끼워도
// 컨테이너 자체에 붙은 리스너는 유지된다.
{
  const container = document.getElementById('trendChart');
  const onPoint = evt => {
    const c = trendPointFromEvent(evt);
    if (c) showTrendPointTooltip(evt, c);
  };
  const offPoint = evt => {
    if (trendPointFromEvent(evt)) hideTooltip();
  };
  container.addEventListener('pointerover', onPoint);
  container.addEventListener('pointermove', onPoint);
  container.addEventListener('pointerout', offPoint);
  container.addEventListener('focusin', onPoint);
  container.addEventListener('focusout', offPoint);
}

function hourlySegmentFromEvent(evt) {
  if (!hourlyChartState || !evt.target || !evt.target.closest) return null;
  return evt.target.closest('rect.hourly-segment');
}

function showHourlyTooltip(evt, segment) {
  const hour = Number(segment.getAttribute('data-hour'));
  const key = hourlyChartState.keys[Number(segment.getAttribute('data-key-idx'))];
  const value = hourlyChartState.byHourSeries[hour][key] || 0;
  const total = hourlyChartState.hourTotals[hour];
  const nextHour = (hour + 1) % 24;
  showTooltip(
    evt,
    `<b>${escapeHtml(key)}</b><br>${String(hour).padStart(2, '0')}:00–${String(nextHour).padStart(2, '0')}:00` +
      `<br>${fmtKo(value)} 토큰 (시간대 합계 ${fmtKo(total)})`,
  );
}

{
  const container = document.getElementById('hourlyChart');
  const onSegment = evt => {
    const segment = hourlySegmentFromEvent(evt);
    if (segment) showHourlyTooltip(evt, segment);
  };
  const offSegment = evt => {
    if (hourlySegmentFromEvent(evt)) hideTooltip();
  };
  container.addEventListener('pointerover', onSegment);
  container.addEventListener('pointermove', onSegment);
  container.addEventListener('pointerout', offSegment);
  container.addEventListener('focusin', onSegment);
  container.addEventListener('focusout', offSegment);
}

function modelSliceFromEvent(evt) {
  if (!modelChartState || !evt.target || !evt.target.closest) return null;
  return evt.target.closest('path.slice');
}

function showModelSliceTooltip(evt, s) {
  const { entries, total, costByModel } = modelChartState;
  const i = Number(s.getAttribute('data-entry-idx'));
  const [model, val] = entries[i];
  const pct = ((val / total) * 100).toFixed(1);
  const summary = costByModel.get(model);
  showTooltip(evt, `<b>${escapeHtml(model)}</b><br>${fmtKo(val)} 토큰 (${pct}%)<br>예상 비용: ${fmtUsdPartial(summary.cost, summary.partial)}`);
}

{
  const container = document.getElementById('modelChart');
  const onSlice = evt => {
    const s = modelSliceFromEvent(evt);
    if (s) showModelSliceTooltip(evt, s);
  };
  const offSlice = evt => {
    if (modelSliceFromEvent(evt)) hideTooltip();
  };
  container.addEventListener('pointerover', onSlice);
  container.addEventListener('pointermove', onSlice);
  container.addEventListener('pointerout', offSlice);
  container.addEventListener('focusin', onSlice);
  container.addEventListener('focusout', offSlice);
}

export function renderTrendChart(rows) {
  const container = document.getElementById('trendChart');
  const legend = document.getElementById('trendLegend');
  container.replaceChildren();
  legend.replaceChildren();
  if (rows.length === 0) {
    trendChartState = null;
    container.replaceChildren(emptyNote('데이터가 없습니다.'));
    return;
  }

  // date -> series -> total_tokens (기록이 없는 조합은 undefined로 남겨 "데이터 없음"과 "0"을 구분한다)
  const rowsByDate = Map.groupBy(rows, r => r.date);
  const dates = [...rowsByDate.keys()].sort();
  const keys = [...new Set(rows.map(seriesKey))].sort();
  const byDateSeries = {};
  for (const d of dates) {
    byDateSeries[d] = {};
    for (const [k, group] of Map.groupBy(rowsByDate.get(d), seriesKey)) {
      byDateSeries[d][k] = group.reduce((sum, r) => sum + r.total_tokens, 0);
    }
  }
  trendChartState = { dates, keys, byDateSeries };

  // 날짜가 많을수록 포인트 간격을 좁혀 SVG 전체 폭을 억제한다
  // (365일 × 60px ≈ 22,000px짜리 SVG는 렌더링/스크롤이 무겁다).
  const pxPerDate = dates.length <= 31 ? 60 : dates.length <= 120 ? 24 : 10;
  const pointRadius = dates.length <= 120 ? 3 : 2;
  // 시리즈는 겹쳐서(overlay) 그리므로 Y축 최댓값은 "개별 시리즈 값의 최댓값"이어야 한다.
  // (합계로 계산하면 스택 차트용 스케일이 되어 모든 선이 하단에 압축되어 보인다.)
  const maxVal = Math.max(1, ...dates.flatMap(d =>
    keys.map(k => byDateSeries[d][k] || 0)
  ));
  const gridSteps = 4;
  const yLabels = compactYAxisLabels(maxVal, gridSteps);
  const defaultPadL = 56;
  const padL = yAxisPadding(yLabels, defaultPadL);
  const width = Math.max(600, dates.length * pxPerDate) + (padL - defaultPadL);
  const height = 260;
  const padR = 16, padT = 12, padB = 28;
  const plotW = width - padL - padR;
  const plotH = height - padT - padB;

  const xStep = dates.length > 1 ? plotW / (dates.length - 1) : 0;
  const x = i => padL + (dates.length > 1 ? i * xStep : plotW / 2);
  const yFor = v => padT + plotH - (v / maxVal) * plotH;

  const chartTitle = `일별 토큰 사용량 추이 (${keys.length}개 시리즈, ${dates.length}일)`;
  let svg = `<svg viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" style="display:block" role="img" aria-label="${escapeHtml(chartTitle)}"><title>${escapeHtml(chartTitle)}</title>`;

  // gridlines + y labels
  for (let i = 0; i <= gridSteps; i++) {
    const v = (maxVal / gridSteps) * i;
    const y = yFor(v);
    svg += `<line x1="${padL}" y1="${y}" x2="${width - padR}" y2="${y}" stroke="var(--grid-line)" stroke-width="1"/>`;
    svg += `<text x="${padL - 8}" y="${y + 3}" text-anchor="end">${yLabels[i]}</text>`;
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
    const color = seriesColor(k, keyIdx);
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
    const patternIndex = seriesColorIndices.get(k) ?? keyIdx;
    const { dash } = LINE_PATTERNS[patternIndex % LINE_PATTERNS.length];
    svg += `<path d="${path.trim()}" fill="none" stroke="${color}" stroke-width="2.5" stroke-linecap="round"` +
      (dash ? ` stroke-dasharray="${dash}"` : '') + '/>';
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

  const fragment = document.createDocumentFragment();
  keys.forEach((k, keyIdx) => {
    const color = seriesColor(k, keyIdx);
    const item = document.createElement('div');
    item.className = 'legend-item';
    const line = document.createElement('span');
    line.className = 'swatch-line';
    line.style.borderColor = color;
    const patternIndex = seriesColorIndices.get(k) ?? keyIdx;
    line.style.borderStyle = LINE_PATTERNS[patternIndex % LINE_PATTERNS.length].borderStyle;
    item.append(line, document.createTextNode(k));
    fragment.appendChild(item);
  });
  legend.replaceChildren(fragment);
}

export function renderHourlyChart(rows) {
  const container = document.getElementById('hourlyChart');
  const legend = document.getElementById('hourlyLegend');
  container.replaceChildren();
  legend.replaceChildren();
  if (rows.length === 0) {
    hourlyChartState = null;
    container.replaceChildren(emptyNote('데이터가 없습니다.'));
    return;
  }

  const hours = Array.from({ length: 24 }, (_, i) => i);
  const keys = [...new Set(rows.map(seriesKey))].sort();
  const byHourSeries = hours.map(() => ({}));
  for (const row of rows) {
    const date = new Date(row.hour);
    if (Number.isNaN(date.getTime())) continue;
    const hour = date.getHours();
    const key = seriesKey(row);
    byHourSeries[hour][key] = (byHourSeries[hour][key] || 0) + row.total_tokens;
  }
  const hourTotals = byHourSeries.map(values =>
    Object.values(values).reduce((sum, value) => sum + value, 0)
  );
  const maxVal = Math.max(1, ...hourTotals);
  hourlyChartState = { keys, byHourSeries, hourTotals };

  const gridSteps = 4;
  const yLabels = compactYAxisLabels(maxVal, gridSteps);
  const defaultPadL = 56;
  const padL = yAxisPadding(yLabels, defaultPadL);
  const width = 720 + (padL - defaultPadL), height = 260;
  const padR = 16, padT = 12, padB = 28;
  const plotW = width - padL - padR, plotH = height - padT - padB;
  const band = plotW / 24, barW = Math.max(4, band - 5);
  const yFor = value => padT + plotH - (value / maxVal) * plotH;
  const timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'local';
  const chartTitle = `시간대별 토큰 사용량 (${timeZone}, ${keys.length}개 시리즈)`;
  let svg = `<svg viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" style="display:block" role="img" aria-label="${escapeHtml(chartTitle)}">`;

  for (let i = 0; i <= gridSteps; i++) {
    const value = (maxVal / gridSteps) * i;
    const y = yFor(value);
    svg += `<line x1="${padL}" y1="${y}" x2="${width - padR}" y2="${y}" stroke="var(--grid-line)" stroke-width="1"/>`;
    svg += `<text x="${padL - 8}" y="${y + 3}" text-anchor="end">${yLabels[i]}</text>`;
  }

  hours.forEach(hour => {
    const x = padL + hour * band + (band - barW) / 2;
    let cumulative = 0;
    keys.forEach((key, keyIdx) => {
      const value = byHourSeries[hour][key] || 0;
      if (value <= 0) return;
      const bottom = yFor(cumulative);
      cumulative += value;
      const top = yFor(cumulative);
      const label = `${key} — ${String(hour).padStart(2, '0')}:00: ${fmtKo(value)} 토큰`;
      svg += `<rect x="${x.toFixed(1)}" y="${top.toFixed(1)}" width="${barW.toFixed(1)}" height="${Math.max(0, bottom - top).toFixed(1)}" ` +
        `fill="${seriesColor(key, keyIdx)}" stroke="var(--panel)" stroke-width="1" class="hourly-segment" tabindex="0" data-hour="${hour}" data-key-idx="${keyIdx}" aria-label="${escapeHtml(label)}"></rect>`;
    });
    if (hour % 2 === 0) {
      svg += `<text x="${(x + barW / 2).toFixed(1)}" y="${height - 8}" text-anchor="middle">${String(hour).padStart(2, '0')}</text>`;
    }
  });
  svg += '</svg>';
  container.innerHTML = svg;

  const fragment = document.createDocumentFragment();
  keys.forEach((key, keyIdx) => {
    const item = document.createElement('div');
    item.className = 'legend-item';
    item.append(swatch(seriesColor(key, keyIdx)), document.createTextNode(key));
    fragment.appendChild(item);
  });
  legend.replaceChildren(fragment);
}

export function renderModelChart(rows) {
  const container = document.getElementById('modelChart');
  const legend = document.getElementById('modelLegend');
  container.replaceChildren();
  legend.replaceChildren();
  if (rows.length === 0) {
    modelChartState = null;
    container.replaceChildren(emptyNote('데이터가 없습니다.'));
    return;
  }

  const byModelUsage = new Map();
  for (const [model, group] of Map.groupBy(rows, r => r.model)) {
    const u = { input: 0, cached: 0, creation: 0, output: 0, total: 0, cost: 0, hasCost: false, hasMissingCost: false };
    for (const r of group) {
      u.total += r.total_tokens;
      u.input += r.input_tokens;
      u.cached += r.cached_input_tokens;
      u.creation += (r.cache_creation_input_tokens || 0);
      u.output += r.output_tokens;
      const cost = rowCostUsd(r);
      if (cost != null) { u.cost += cost; u.hasCost = true; }
      else u.hasMissingCost = true;
    }
    byModelUsage.set(model, u);
  }
  const rawEntries = [...byModelUsage].map(([model, u]) => [model, u.total]).sort((a, b) => b[1] - a[1]);
  const totalTokens = rawEntries.reduce((s, [, v]) => s + v, 0);
  const total = totalTokens || 1;
  const totalCost = [...byModelUsage.values()].reduce((s, u) => s + (modelCostUsd(u) || 0), 0);
  const totalHasCost = [...byModelUsage.values()].some(u => u.hasCost);
  const totalHasMissingCost = [...byModelUsage.values()].some(u => u.hasMissingCost);

  // 설정된 최대 항목 수를 넘는 모델은 마지막 "기타" 항목으로 묶는다.
  const OTHER_LABEL = '기타';
  const main = [];
  let otherVal = 0;
  // "기타"는 여러 실모델의 묶음이라 findPricing('기타')가 실패한다. 개별 모델 비용을
  // 여기서 미리 합산해 두고, 툴팁/범례에서 다시 계산하지 않고 이 값을 쓴다.
  const otherUsage = { input: 0, cached: 0, creation: 0, output: 0, cost: 0, hasCost: false, hasMissingCost: false };
  const keepCount = rawEntries.length > modelChartMaxItems ? modelChartMaxItems - 1 : rawEntries.length;
  for (const [index, [model, val]] of rawEntries.entries()) {
    if (index >= keepCount) {
      otherVal += val;
      const u = byModelUsage.get(model);
      otherUsage.input += u.input;
      otherUsage.cached += u.cached;
      otherUsage.creation += u.creation;
      otherUsage.output += u.output;
      const c = modelCostUsd(u);
      if (c != null) { otherUsage.cost += c; otherUsage.hasCost = true; }
      if (u.hasMissingCost) otherUsage.hasMissingCost = true;
    } else {
      main.push([model, val]);
    }
  }
  const entries = otherVal > 0 ? [...main, [OTHER_LABEL, otherVal]] : main;
  if (otherVal > 0) byModelUsage.set(OTHER_LABEL, otherUsage);

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
    const color = modelColor(model, i);
    const path = frac >= 0.9999
      ? `M ${cx} ${cy - r} A ${r} ${r} 0 1 1 ${cx - 0.01} ${cy - r} Z`
      : `M ${cx} ${cy} L ${x1} ${y1} A ${r} ${r} 0 ${largeArc} 1 ${x2} ${y2} Z`;
    const pct = (frac * 100).toFixed(1);
    const label = `${model}: ${fmtKo(val)} 토큰 (${pct}%)`;
    svg += `<path d="${path}" fill="${color}" stroke="var(--panel)" stroke-width="3" class="slice" ` +
      `tabindex="0" data-entry-idx="${i}" aria-label="${escapeHtml(label)}"><title>${escapeHtml(label)}</title></path>`;
    angle = nextAngle;
  });
  svg += `</svg>`;
  container.innerHTML = svg;

  // "기타"는 findPricing 매칭이 안 되므로 위에서 미리 합산한 비용(otherUsage)을 쓰고,
  // 실모델은 그대로 계산한다. 비용이 하나도 없으면 null, 일부만 계산됐으면
  // partial=true로 전달해 완전한 합계처럼 보이지 않게 한다.
  const costByModel = new Map(entries.map(([model]) => {
    const u = model === OTHER_LABEL ? otherUsage : byModelUsage.get(model);
    return [model, { cost: modelCostUsd(u), partial: u.hasCost && u.hasMissingCost }];
  }));
  modelChartState = { entries, total, costByModel };

  const totalItem = document.createElement('div');
  totalItem.className = 'legend-item legend-total';
  const totalText = document.createElement('b');
  const totalCostLabel = totalHasMissingCost ? '총 예상 비용(부분 합계)' : '총 예상 비용';
  totalText.textContent = `${totalCostLabel}: ${fmtUsdPartial(totalHasCost ? totalCost : null, totalHasCost && totalHasMissingCost)} · 총 소모 토큰: ${fmtKo(totalTokens)}`;
  totalText.title = totalHasMissingCost
    ? `일부 모델의 단가가 없습니다. 총 소모 토큰: ${totalTokens.toLocaleString('ko-KR')}`
    : `총 소모 토큰: ${totalTokens.toLocaleString('ko-KR')}`;
  totalItem.appendChild(totalText);

  const fragment = document.createDocumentFragment();
  fragment.appendChild(totalItem);
  entries.forEach(([model, val], i) => {
    const color = modelColor(model, i);
    const pct = ((val / total) * 100).toFixed(1);
    const cost = costByModel.get(model);
    const item = document.createElement('div');
    item.className = 'legend-item';
    item.append(swatch(color), document.createTextNode(`${model} (${pct}%) — ${fmtUsdPartial(cost.cost, cost.partial)}`));
    fragment.appendChild(item);
  });
  legend.replaceChildren(fragment);
}

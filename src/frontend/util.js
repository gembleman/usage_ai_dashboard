export const PALETTE = ['#7c9eff', '#4fd1a5', '#f5c26b', '#f47174', '#b98cf0', '#5bc8e0', '#e08fd0', '#9fd15a'];
const NUMBER_FORMAT = new Intl.NumberFormat('en-US');
const USD_FORMAT = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});
const KO_TIME_FORMAT = new Intl.DateTimeFormat('ko-KR', { timeStyle: 'medium' });
const KO_DATE_TIME_FORMAT = new Intl.DateTimeFormat('ko-KR', { dateStyle: 'medium', timeStyle: 'medium' });
const KO_DURATION_FORMAT = new Intl.DurationFormat('ko-KR', { style: 'long' });

export const fmt = n => NUMBER_FORMAT.format(n);
export const fmtTime = value => KO_TIME_FORMAT.format(value instanceof Date ? value : new Date(value));
export const fmtDateTime = value => KO_DATE_TIME_FORMAT.format(value instanceof Date ? value : new Date(value));

// Human-readable source labels. Codex and Claude Code accounts can share a
// display name (e.g. "user01"), so the source badge disambiguates them.
export const SOURCE_LABELS = {
  codex: 'Codex',
  claude_code: 'Claude Code',
};

// innerHTML에 삽입하기 전에 사용자/로그 기반 문자열(계정명, 모델명 등)을 이스케이프한다.
const ESCAPE_HTML_MAP = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
export function escapeHtml(str) {
  return String(str == null ? '' : str).replace(/[&<>"']/g, ch => ESCAPE_HTML_MAP[ch]);
}

// 큰 숫자를 억/만/천 단위의 한국어 표기로 변환 (예: 12345678 -> "1,234만 5,678")
export function fmtKo(n) {
  n = Math.round(n);
  if (n < 1000) return String(n);
  const neg = n < 0;
  n = Math.abs(n);
  const eok = Math.floor(n / 100000000);
  const man = Math.floor((n % 100000000) / 10000);
  const rest = n % 10000;
  const parts = [];
  if (eok > 0) parts.push(`${fmt(eok)}억`);
  if (man > 0) parts.push(`${eok > 0 ? String(man).padStart(4, '0') : fmt(man)}만`);
  if (rest > 0 || parts.length === 0) parts.push(`${man > 0 ? String(rest).padStart(4, '0') : fmt(rest)}`);
  return (neg ? '-' : '') + parts.join(' ');
}

export function fmtDurationKo(sec) {
  if (sec <= 0) return null;
  const d = Math.floor(sec / 86400);
  const h = Math.floor((sec % 86400) / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = Math.floor(sec % 60);
  if (d > 0) return KO_DURATION_FORMAT.format({ days: d, hours: h });
  if (h > 0) return KO_DURATION_FORMAT.format({ hours: h, minutes: m });
  if (m > 0) return KO_DURATION_FORMAT.format({ minutes: m, seconds: s });
  return KO_DURATION_FORMAT.format({ seconds: s });
}

// 모델별 100만 토큰당 가격(USD)은 config.toml에서 로드한다.
// 설정되지 않은 모델은 비용을 계산하지 않는다.
const MODEL_PRICING = {};

// config.toml에서 전달된 가격표를 적용한다.
export function applyModelPricing(prices) {
  for (const [model, pricing] of Object.entries(prices || {})) {
    if (pricing && [
      pricing.input,
      pricing.cached_input,
      pricing.cache_creation_input,
      pricing.output,
    ].every(Number.isFinite)) {
      MODEL_PRICING[model] = {
        input: pricing.input,
        cached_input: pricing.cached_input,
        cache_creation_input: pricing.cache_creation_input,
        output: pricing.output,
      };
    }
  }
}

function findPricing(model) {
  if (!model) return undefined;
  if (MODEL_PRICING[model]) return MODEL_PRICING[model];
  // gpt-5.4보다 gpt-5.4-mini처럼 더 구체적인 ID를 먼저 매칭한다.
  const key = Object.keys(MODEL_PRICING)
    .filter(k => model.includes(k))
    .sort((a, b) => b.length - a.length)[0];
  return key ? MODEL_PRICING[key] : undefined;
}

// 캐시 비용 분리 계산: cache read는 정가 대비 10%, cache creation은 정가 대비 125%.
// 이전에는 둘을 합산해 read 요율만 적용해 ~20% 과소추정됐다.
export function estimateCostUsd(model, inputTokens, cachedInputTokens, cacheCreationInputTokens, outputTokens) {
  const p = findPricing(model);
  if (!p) return null;
  const inCost = (inputTokens / 1e6) * p.input;
  const cachedReadCost = (cachedInputTokens / 1e6) * p.cached_input;
  const cachedCreationCost = ((cacheCreationInputTokens || 0) / 1e6) * p.cache_creation_input;
  const outCost = (outputTokens / 1e6) * p.output;
  return inCost + cachedReadCost + cachedCreationCost + outCost;
}

export const fmtUsd = v => !Number.isFinite(v) ? '—' : USD_FORMAT.format(v);
const tooltip = document.getElementById('tooltip');

export function showTooltip(evt, html) {
  if (typeof tooltip.setHTML === 'function') {
    tooltip.setHTML(html);
  } else {
    tooltip.innerHTML = html;
  }
  if (typeof tooltip.showPopover === 'function') {
    if (!tooltip.matches(':popover-open')) tooltip.showPopover();
  } else {
    tooltip.style.display = 'block';
  }

  // FocusEvent(키보드 포커스)에는 clientX/Y가 없으므로 이벤트 대상 요소의
  // 중심 좌표를 앵커로 폴백한다.
  let anchorX = evt.clientX;
  let anchorY = evt.clientY;
  if (!Number.isFinite(anchorX) || !Number.isFinite(anchorY)) {
    const t = evt.target && evt.target.getBoundingClientRect ? evt.target.getBoundingClientRect() : null;
    anchorX = t ? t.left + t.width / 2 : 0;
    anchorY = t ? t.top + t.height / 2 : 0;
  }

  const margin = 8;
  const rect = tooltip.getBoundingClientRect();
  const vw = window.innerWidth;
  const vh = window.innerHeight;

  let left = anchorX + 14;
  let top = anchorY + 14;

  // 오른쪽/아래쪽 경계를 넘으면 커서 반대편으로 뒤집는다.
  if (left + rect.width + margin > vw) left = anchorX - 14 - rect.width;
  if (top + rect.height + margin > vh) top = anchorY - 14 - rect.height;

  // 그래도 화면 밖이면(작은 뷰포트) 경계 안쪽으로 클램프한다.
  left = Math.max(margin, Math.min(left, vw - rect.width - margin));
  top = Math.max(margin, Math.min(top, vh - rect.height - margin));

  tooltip.style.left = left + 'px';
  tooltip.style.top = top + 'px';
}
export function hideTooltip() {
  if (typeof tooltip.hidePopover === 'function') {
    if (tooltip.matches(':popover-open')) tooltip.hidePopover();
  } else {
    tooltip.style.display = 'none';
  }
}

export function updateWithViewTransition(update) {
  if (!document.startViewTransition || matchMedia('(prefers-reduced-motion: reduce)').matches) {
    update();
  } else {
    document.startViewTransition(update);
  }
}

export function seriesKey(row) {
  return `${row.source}/${row.account}`;
}

export function colorFor(key, keys) {
  const idx = keys.indexOf(key);
  return PALETTE[idx % PALETTE.length];
}

export function emptyNote(message) {
  const div = document.createElement('div');
  div.className = 'empty-note';
  div.textContent = message;
  return div;
}

export function emptyTableRow(colspan, message) {
  const tr = document.createElement('tr');
  const td = document.createElement('td');
  td.colSpan = colspan;
  td.className = 'empty-note';
  td.textContent = message;
  tr.appendChild(td);
  return tr;
}

export function swatch(color) {
  const span = document.createElement('span');
  span.className = 'swatch';
  span.style.background = color;
  return span;
}

let defaultApiTimeoutMs = 30000;
export function setDefaultApiTimeout(seconds) {
  if (Number.isFinite(seconds) && seconds > 0) defaultApiTimeoutMs = seconds * 1000;
}

export async function fetchJson(url, opts = {}) {
  const timeoutMs = opts.timeoutMs ?? defaultApiTimeoutMs;
  const { timeoutMs: _, ...fetchOpts } = opts;
  const timeoutSignal = AbortSignal.timeout(timeoutMs);
  const signal = fetchOpts.signal
    ? AbortSignal.any([fetchOpts.signal, timeoutSignal])
    : timeoutSignal;
  const res = await fetch(url, {
    ...fetchOpts,
    signal,
  });
  if (!res.ok) {
    let detail = '';
    try {
      detail = await res.text();
    } catch (e) {
      // 본문을 읽지 못하면 상태 코드만 사용
    }
    throw new Error(detail ? `${url} -> ${res.status}: ${detail}` : `${url} -> ${res.status}`);
  }
  return res.json();
}

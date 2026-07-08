const PALETTE = ['#7c9eff', '#4fd1a5', '#f5c26b', '#f47174', '#b98cf0', '#5bc8e0', '#e08fd0', '#9fd15a'];
const fmt = n => n.toLocaleString('en-US');

// innerHTML에 삽입하기 전에 사용자/로그 기반 문자열(계정명, 모델명 등)을 이스케이프한다.
const ESCAPE_HTML_MAP = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
function escapeHtml(str) {
  return String(str == null ? '' : str).replace(/[&<>"']/g, ch => ESCAPE_HTML_MAP[ch]);
}

// 큰 숫자를 억/만/천 단위의 한국어 표기로 변환 (예: 12345678 -> "1,234만 5,678")
function fmtKo(n) {
  n = Math.round(n);
  if (n < 1000) return String(n);
  const neg = n < 0;
  n = Math.abs(n);
  const eok = Math.floor(n / 100000000);
  const man = Math.floor((n % 100000000) / 10000);
  const rest = n % 10000;
  const parts = [];
  if (eok > 0) parts.push(`${eok.toLocaleString('en-US')}억`);
  if (man > 0) parts.push(`${eok > 0 ? String(man).padStart(4, '0') : man.toLocaleString('en-US')}만`);
  if (rest > 0 || parts.length === 0) parts.push(`${(man > 0 || eok > 0) ? String(rest).padStart(4, '0') : rest.toLocaleString('en-US')}`);
  return (neg ? '-' : '') + parts.join(' ');
}

// 모델별 100만 토큰당 가격 (USD). 알려지지 않은 모델은 undefined 반환.
const MODEL_PRICING = {
  // Anthropic Claude
  'claude-opus-4-8': { input: 5, output: 25 },
  'claude-opus-4-7': { input: 5, output: 25 },
  'claude-sonnet-5': { input: 3, output: 15 },
  'claude-sonnet-4-6': { input: 3, output: 15 },
  'claude-fable-5': { input: 10, output: 50 },
  'claude-haiku-4-5': { input: 1, output: 5 },
  // OpenAI Codex / GPT
  'gpt-5.5': { input: 5, output: 30 },
  'gpt-5.4': { input: 2.5, output: 15 },
  'gpt-5.4-mini': { input: 0.75, output: 4.5 },
  'gpt-5.4-nano': { input: 0.2, output: 1.25 },
  'gpt-4.1': { input: 2, output: 8 },
};

function findPricing(model) {
  if (!model) return undefined;
  if (MODEL_PRICING[model]) return MODEL_PRICING[model];
  const key = Object.keys(MODEL_PRICING).find(k => model.includes(k) || k.includes(model));
  return key ? MODEL_PRICING[key] : undefined;
}

// 캐시 입력 토큰은 캐시 할인율(정가 대비 10%)로 근사 계산
function estimateCostUsd(model, inputTokens, cachedInputTokens, outputTokens) {
  const p = findPricing(model);
  if (!p) return null;
  const inCost = (inputTokens / 1e6) * p.input;
  const cachedCost = (cachedInputTokens / 1e6) * p.input * 0.1;
  const outCost = (outputTokens / 1e6) * p.output;
  return inCost + cachedCost + outCost;
}

const fmtUsd = v => v == null ? '—' : `$${v.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
const tooltip = document.getElementById('tooltip');

function showTooltip(evt, html) {
  tooltip.innerHTML = html;
  tooltip.style.display = 'block';

  // FocusEvent(키보드 포커스)나 touches가 비어 있는 TouchEvent에는 clientX/Y가
  // 없으므로, 이벤트 대상 요소의 중심 좌표를 앵커로 폴백한다.
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
function hideTooltip() { tooltip.style.display = 'none'; }

function seriesKey(row) {
  return `${row.source}/${row.account}`;
}

function colorFor(key, keys) {
  const idx = keys.indexOf(key);
  return PALETTE[idx % PALETTE.length];
}

async function fetchJson(url, opts) {
  const res = await fetch(url, opts);
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

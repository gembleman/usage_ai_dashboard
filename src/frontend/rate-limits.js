import { SOURCE_LABELS, emptyNote, fmtDateTime, fmtDurationKo } from './util.js';

function meterColor(pct) {
  if (pct >= 90) return 'var(--bad)';
  if (pct >= 70) return 'var(--warn)';
  return 'var(--good)';
}

// data-resets-at(epoch 초)을 가진 모든 카운트다운 요소를 현재 시각 기준으로 갱신.
// 렌더링과 분리된 전역 타이머가 매초 호출하므로, 재렌더링 없이도 시간이 흐른다.
function updateResetCountdowns() {
  if (document.hidden) return; // 백그라운드 탭에서는 갱신 생략 (복귀 시 visibilitychange로 즉시 갱신)
  const now = Date.now() / 1000;
  for (const el of document.querySelectorAll('.rl-countdown')) {
    const remaining = fmtDurationKo(Number(el.dataset.resetsAt) - now);
    if (remaining === null) {
      // 초기화 시각이 지났지만 아직 새 스냅샷을 받지 못한 상태.
      el.textContent = '초기화됨 — 새로고침으로 갱신';
      el.classList.add('rl-countdown-done');
    } else {
      el.textContent = `${remaining} 남음`;
      el.classList.remove('rl-countdown-done');
    }
  }
}

setInterval(updateResetCountdowns, 1000);
document.addEventListener('visibilitychange', () => {
  if (!document.hidden) updateResetCountdowns();
});

function div(className, text) {
  const el = document.createElement('div');
  if (className) el.className = className;
  if (text != null) el.textContent = text;
  return el;
}

function span(className, text) {
  const el = document.createElement('span');
  if (className) el.className = className;
  if (text != null) el.textContent = text;
  return el;
}

function tag(text) {
  return span('tag', text);
}

function labelRow(left, right) {
  const row = div('rl-window-label');
  row.append(span('', left), span('', right));
  return row;
}

function meter(pct, ariaLabel) {
  const outer = div('meter');
  outer.setAttribute('role', 'img');
  outer.setAttribute('aria-label', ariaLabel);
  const fill = div('meter-fill');
  fill.style.width = `${pct}%`;
  fill.style.background = meterColor(pct);
  outer.appendChild(fill);
  return outer;
}

function rateWindow(label, w) {
  if (!w) return null;
  const pct = Math.min(100, w.used_percent);
  const wrap = div('rl-window');
  wrap.append(
    labelRow(`${label} (${w.window_minutes}분 윈도우)`, `${w.used_percent.toFixed(1)}% 사용`),
    meter(pct, `${label} 사용률 ${w.used_percent.toFixed(1)}%`)
  );

  // resets_at이 0이면 초기화 시각 정보가 없는 것 (백엔드가 알 수 없을 때
  // 0으로 폴백) — 1970년 표기와 오탐 카운트다운 대신 행 자체를 생략한다.
  if (w.resets_at) {
    const resetRow = div('rl-window-label');
    const countdown = span('rl-countdown');
    countdown.dataset.resetsAt = String(w.resets_at);
    resetRow.append(countdown, span('', `${fmtDateTime(w.resets_at * 1000)} 초기화`));
    wrap.appendChild(resetRow);
  }
  return wrap;
}

function extraUsageWindow(extra) {
  const fmtCredit = (v, digits) => (v == null ? '?' : v.toFixed(digits));
  const wrap = div('rl-window');
  wrap.appendChild(labelRow('추가 사용 크레딧', `${fmtCredit(extra.used_credits, 2)} / ${fmtCredit(extra.monthly_limit, 0)} 크레딧`));

  if (extra.utilization != null) {
    const pct = Math.min(100, extra.utilization);
    wrap.append(
      meter(pct, `추가 사용 크레딧 사용률 ${extra.utilization.toFixed(1)}%`),
      labelRow('', `${extra.utilization.toFixed(1)}% 사용`)
    );
  }
  return wrap;
}

function rateLimitCard(snap) {
  const card = div('rl-card');
  const observed = fmtDateTime(snap.observed_at);
  const sourceLabel = SOURCE_LABELS[snap.source] || snap.source || '';
  // Claude Code windows come from the OAuth usage API and use different
  // labels than Codex's 5h/7d rate-limit windows.
  const isClaude = snap.source === 'claude_code';
  const primaryLabel = isClaude ? '세션 / 5시간' : '1차 / 5시간';
  const secondaryLabel = isClaude ? '주간 / 7일' : '2차 / 7일';

  const head = div('rl-head');
  const account = span('rl-account');
  account.append(tag(sourceLabel), document.createTextNode(snap.account || ''));
  if (snap.plan_type) account.appendChild(tag(snap.plan_type));
  head.append(account, span('rl-observed', `${observed} 관측`));
  card.appendChild(head);

  for (const windowEl of [
    rateWindow(primaryLabel, snap.primary),
    rateWindow(secondaryLabel, snap.secondary),
    rateWindow('Opus 주간', snap.seven_day_opus),
    rateWindow('Sonnet 주간', snap.seven_day_sonnet),
  ]) {
    if (windowEl) card.appendChild(windowEl);
  }

  if (snap.extra_usage) card.appendChild(extraUsageWindow(snap.extra_usage));

  if (snap.rate_limit_reached_type) {
    const reached = div('rl-window-label', `한도 도달 유형: ${snap.rate_limit_reached_type}`);
    reached.style.color = 'var(--bad)';
    card.appendChild(reached);
  }

  return card;
}

export function renderRateLimits(snapshots) {
  const container = document.getElementById('rateLimits');
  if (snapshots.length === 0) {
    container.replaceChildren(emptyNote('요청 한도 스냅샷이 없습니다.'));
    return;
  }

  // Codex first, then Claude Code; within a source, by account name.
  const order = { codex: 0, claude_code: 1 };
  const sorted = [...snapshots].sort((a, b) => {
    const s = (order[a.source] ?? 9) - (order[b.source] ?? 9);
    return s !== 0 ? s : (a.account || '').localeCompare(b.account || '');
  });

  const fragment = document.createDocumentFragment();
  for (const snap of sorted) fragment.appendChild(rateLimitCard(snap));
  container.replaceChildren(fragment);

  // 다음 타이머 틱(최대 1초)을 기다리지 않고 즉시 카운트다운을 채운다.
  updateResetCountdowns();
}

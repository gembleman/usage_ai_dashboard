function meterColor(pct) {
  if (pct >= 90) return 'var(--bad)';
  if (pct >= 70) return 'var(--warn)';
  return 'var(--good)';
}

// Human-readable source labels. Codex and Claude Code accounts can share a
// display name (e.g. "user01"), so the source badge disambiguates them.
const SOURCE_LABELS = {
  codex: 'Codex',
  claude_code: 'Claude Code',
};

function renderRateLimits(snapshots) {
  const container = document.getElementById('rateLimits');
  container.innerHTML = '';
  if (snapshots.length === 0) {
    container.innerHTML = '<div class="empty-note">요청 한도 스냅샷이 없습니다.</div>';
    return;
  }
  // Codex first, then Claude Code; within a source, by account name.
  const order = { codex: 0, claude_code: 1 };
  const sorted = [...snapshots].sort((a, b) => {
    const s = (order[a.source] ?? 9) - (order[b.source] ?? 9);
    return s !== 0 ? s : (a.account || '').localeCompare(b.account || '');
  });

  for (const snap of sorted) {
    const card = document.createElement('div');
    card.className = 'rl-card';
    const observed = new Date(snap.observed_at).toLocaleString('ko-KR');
    const sourceLabel = SOURCE_LABELS[snap.source] || snap.source || '';
    // Claude Code windows come from the OAuth usage API and use different
    // labels than Codex's 5h/7d rate-limit windows.
    const isClaude = snap.source === 'claude_code';
    const primaryLabel = isClaude ? '세션 / 5시간' : '1차 / 5시간';
    const secondaryLabel = isClaude ? '주간 / 7일' : '2차 / 7일';

    let html = `<div class="rl-head">
      <span class="rl-account"><span class="tag">${escapeHtml(sourceLabel)}</span>${escapeHtml(snap.account)}${snap.plan_type ? `<span class="tag">${escapeHtml(snap.plan_type)}</span>` : ''}</span>
      <span class="rl-observed">${escapeHtml(observed)} 관측</span>
    </div>`;

    // 퍼센트 텍스트를 별도로 표기해 색상(정상/경고/위험)에만 의존하지 않도록 한다.
    const windowHtml = (label, w) => {
      if (!w) return '';
      const pct = Math.min(100, w.used_percent);
      const resets = new Date(w.resets_at * 1000).toLocaleString('ko-KR');
      return `<div class="rl-window">
        <div class="rl-window-label"><span>${escapeHtml(label)} (${w.window_minutes}분 윈도우)</span><span>${w.used_percent.toFixed(1)}% 사용</span></div>
        <div class="meter" role="img" aria-label="${escapeHtml(label)} 사용률 ${w.used_percent.toFixed(1)}%"><div class="meter-fill" style="width:${pct}%;background:${meterColor(pct)}"></div></div>
        <div class="rl-window-label"><span></span><span>${escapeHtml(resets)} 초기화</span></div>
      </div>`;
    };

    html += windowHtml(primaryLabel, snap.primary);
    html += windowHtml(secondaryLabel, snap.secondary);
    // Claude Code 전용: 모델별 주간 한도 (플랜에 없으면 null → 행 생략).
    html += windowHtml('Opus 주간', snap.seven_day_opus);
    html += windowHtml('Sonnet 주간', snap.seven_day_sonnet);
    // Claude Code 전용: 추가 사용 크레딧 (활성화된 계정만 내려옴).
    if (snap.extra_usage) {
      const e = snap.extra_usage;
      const fmt = (v, digits) => (v == null ? '?' : v.toFixed(digits));
      let extraHtml = `<div class="rl-window">
        <div class="rl-window-label"><span>추가 사용 크레딧</span><span>${fmt(e.used_credits, 2)} / ${fmt(e.monthly_limit, 0)} 크레딧</span></div>`;
      if (e.utilization != null) {
        const pct = Math.min(100, e.utilization);
        extraHtml += `<div class="meter" role="img" aria-label="추가 사용 크레딧 사용률 ${e.utilization.toFixed(1)}%"><div class="meter-fill" style="width:${pct}%;background:${meterColor(pct)}"></div></div>
        <div class="rl-window-label"><span></span><span>${e.utilization.toFixed(1)}% 사용</span></div>`;
      }
      extraHtml += '</div>';
      html += extraHtml;
    }
    if (snap.rate_limit_reached_type) {
      html += `<div class="rl-window-label" style="color:var(--bad)">한도 도달 유형: ${escapeHtml(snap.rate_limit_reached_type)}</div>`;
    }
    card.innerHTML = html;
    container.appendChild(card);
  }
}

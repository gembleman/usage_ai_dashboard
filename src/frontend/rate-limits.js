function meterColor(pct) {
  if (pct >= 90) return 'var(--bad)';
  if (pct >= 70) return 'var(--warn)';
  return 'var(--good)';
}

function renderRateLimits(snapshots) {
  const container = document.getElementById('rateLimits');
  container.innerHTML = '';
  if (snapshots.length === 0) {
    container.innerHTML = '<div class="empty-note">Codex 요청 한도 스냅샷이 없습니다.</div>';
    return;
  }
  for (const snap of snapshots) {
    const card = document.createElement('div');
    card.className = 'rl-card';
    const observed = new Date(snap.observed_at).toLocaleString('ko-KR');
    let html = `<div class="rl-head">
      <span class="rl-account">${snap.account}${snap.plan_type ? `<span class="tag">${snap.plan_type}</span>` : ''}</span>
      <span class="rl-observed">${observed} 관측</span>
    </div>`;

    const windowHtml = (label, w) => {
      if (!w) return '';
      const pct = Math.min(100, w.used_percent);
      const resets = new Date(w.resets_at * 1000).toLocaleString('ko-KR');
      return `<div class="rl-window">
        <div class="rl-window-label"><span>${label} (${w.window_minutes}분 윈도우)</span><span>${w.used_percent.toFixed(1)}% 사용</span></div>
        <div class="meter"><div class="meter-fill" style="width:${pct}%;background:${meterColor(pct)}"></div></div>
        <div class="rl-window-label"><span></span><span>${resets} 초기화</span></div>
      </div>`;
    };

    html += windowHtml('1차 / 5시간', snap.primary);
    html += windowHtml('2차 / 7일', snap.secondary);
    if (snap.rate_limit_reached_type) {
      html += `<div class="rl-window-label" style="color:var(--bad)">한도 도달 유형: ${snap.rate_limit_reached_type}</div>`;
    }
    card.innerHTML = html;
    container.appendChild(card);
  }
}

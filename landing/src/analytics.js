// 쿠키 없는 방문 분석(Umami). ID가 없으면 아무것도 넣지 않고 track()은 조용히 끝난다.
import { UMAMI_ID, UMAMI_SRC } from './config.js';

export function initAnalytics() {
  if (!UMAMI_ID) return false;
  const s = document.createElement('script');
  s.defer = true; s.src = UMAMI_SRC;
  s.dataset.websiteId = UMAMI_ID;
  s.dataset.autoTrack = 'true';
  document.head.appendChild(s);
  return true;
}

export function track(name, props = {}) {
  try { window.umami?.track?.(name, props); } catch { /* 차단되거나 없으면 무시 */ }
}

/** data-track="이름:값" 속성이 있는 요소의 클릭을 한 번에 묶는다 */
export function bindTracking(root = document) {
  root.addEventListener('click', event => {
    const el = event.target.closest('[data-track]');
    if (!el) return;
    const [name, place] = el.dataset.track.split(':');
    track(name, place ? { place, lang: document.documentElement.lang } : { lang: document.documentElement.lang });
  }, { passive: true });
}

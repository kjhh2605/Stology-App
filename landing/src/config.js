// 배포 설정. 값이 비어 있으면 해당 기능을 조용히 끈다.
// 빌드 시 환경변수: VITE_SITE_URL(절대 URL, hreflang·canonical·og:url용), VITE_UMAMI_ID(Umami 웹사이트 ID), VITE_UMAMI_SRC(스크립트 주소)
export const SITE_URL = (import.meta.env.VITE_SITE_URL || '').replace(/\/$/, '');
export const UMAMI_ID = import.meta.env.VITE_UMAMI_ID || '';
export const UMAMI_SRC = import.meta.env.VITE_UMAMI_SRC || 'https://cloud.umami.is/script.js';

// 버전을 올릴 때 app/scripts/release.sh가 만든 GitHub 릴리스 주소로 바꾼다.
export const DOWNLOAD_VERSION = '0.1.5';
export const DOWNLOAD_URL = `https://github.com/kjhh2605/Stology-App/releases/download/v${DOWNLOAD_VERSION}/Stology-Local-${DOWNLOAD_VERSION}-arm64.dmg`;
export const RELEASES_URL = 'https://github.com/kjhh2605/Stology-App/releases';
export const DISCORD_URL = 'https://discord.gg/SJ6cGyjEF';

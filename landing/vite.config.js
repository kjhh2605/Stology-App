import { defineConfig, loadEnv } from 'vite';
import { resolve } from 'node:path';

// 한국어 `/`와 영어 `/en/`을 각각의 HTML로 만든다. VITE_SITE_URL이 있으면 canonical·hreflang·og:url·og:image(절대 URL)를 넣는다.
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), 'VITE_');
  const site = (env.VITE_SITE_URL || '').replace(/\/$/, '');
  return {
    build: { rollupOptions: { input: { main: resolve(__dirname, 'index.html'), en: resolve(__dirname, 'en/index.html'), notfound: resolve(__dirname, '404.html') } } },
    plugins: [{
      name: 'stology-locale-head',
      transformIndexHtml(html, ctx) {
        const isEn = /[\\/]en[\\/]index\.html$/.test(ctx.filename) || ctx.path?.startsWith('/en/');
        const page = isEn ? '/en/' : '/';
        const og = isEn ? '/og-en.png' : '/og-ko.png';
        const tags = [{ tag: 'meta', attrs: { property: 'og:image', content: site ? site + og : og }, injectTo: 'head' }, { tag: 'meta', attrs: { property: 'og:image:width', content: '1200' }, injectTo: 'head' }, { tag: 'meta', attrs: { property: 'og:image:height', content: '630' }, injectTo: 'head' }, { tag: 'meta', attrs: { name: 'twitter:image', content: site ? site + og : og }, injectTo: 'head' }];
        if (site) {
          tags.push(
            { tag: 'link', attrs: { rel: 'canonical', href: site + page }, injectTo: 'head' },
            { tag: 'link', attrs: { rel: 'alternate', hreflang: 'ko', href: site + '/' }, injectTo: 'head' },
            { tag: 'link', attrs: { rel: 'alternate', hreflang: 'en', href: site + '/en/' }, injectTo: 'head' },
            { tag: 'link', attrs: { rel: 'alternate', hreflang: 'x-default', href: site + '/' }, injectTo: 'head' },
            { tag: 'meta', attrs: { property: 'og:url', content: site + page }, injectTo: 'head' },
          );
        }
        return tags;
      },
    }],
  };
});

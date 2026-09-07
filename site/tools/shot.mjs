/**
 * Full-page screenshot over CDP. Chrome's `--screenshot` flag only captures the
 * viewport, which is useless for a page whose hero is 100svh.
 *
 *   node tools/shot.mjs <url> <out.png> [width] [height] [clipTop] [clipHeight]
 */
import { spawn } from 'node:child_process';
import { writeFileSync, rmSync } from 'node:fs';
import { setTimeout as sleep } from 'node:timers/promises';

const [url, out, w = '1440', h = '900', clipTop, clipHeight] = process.argv.slice(2);
const PORT = 9333;
const PROFILE = '/tmp/puck-shot-profile';

const chrome = spawn(
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  [
    '--headless=new',
    '--disable-gpu',
    '--hide-scrollbars',
    '--no-first-run',
    '--no-default-browser-check',
    `--remote-debugging-port=${PORT}`,
    `--user-data-dir=${PROFILE}`,
    'about:blank',
  ],
  { stdio: 'ignore' },
);

let target;
for (let i = 0; i < 60; i++) {
  await sleep(250);
  try {
    const list = await fetch(`http://127.0.0.1:${PORT}/json/list`).then((r) => r.json());
    target = list.find((t) => t.type === 'page');
    if (target) break;
  } catch {}
}
if (!target) throw new Error('chrome did not come up');

const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((res) => ws.addEventListener('open', res));

let id = 0;
const pending = new Map();
ws.addEventListener('message', (e) => {
  const msg = JSON.parse(e.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg.result);
    pending.delete(msg.id);
  }
});
const send = (method, params = {}) =>
  new Promise((res) => {
    const n = ++id;
    pending.set(n, res);
    ws.send(JSON.stringify({ id: n, method, params }));
  });

await send('Emulation.setDeviceMetricsOverride', {
  width: Number(w),
  height: Number(h),
  deviceScaleFactor: 1,
  mobile: false,
});
await send('Page.enable');
await send('Page.navigate', { url });
await send('Runtime.enable');
await sleep(1500);

// Walk the page so lazy images and IntersectionObserver reveals fire, then
// return to the top. captureBeyondViewport does neither on its own.
const pageH = (await send('Runtime.evaluate', {
  expression: 'document.documentElement.scrollHeight',
  returnByValue: true,
})).result.value;
for (let y = 0; y < pageH; y += Number(h) * 0.75) {
  await send('Runtime.evaluate', { expression: `window.scrollTo(0, ${y})` });
  await sleep(200);
}
await send('Runtime.evaluate', { expression: 'window.scrollTo(0, 0)' });
await sleep(1200);

const metrics = await send('Page.getLayoutMetrics');
const full = metrics.cssContentSize ?? metrics.contentSize;
const clip =
  clipTop !== undefined
    ? { x: 0, y: Number(clipTop), width: Number(w), height: Number(clipHeight), scale: 1 }
    : { x: 0, y: 0, width: Number(w), height: Math.ceil(full.height), scale: 1 };

const shot = await send('Page.captureScreenshot', {
  format: 'png',
  captureBeyondViewport: true,
  clip,
});
writeFileSync(out, Buffer.from(shot.data, 'base64'));
console.log(out, `${clip.width}x${clip.height}`, `page height ${Math.round(full.height)}`);

ws.close();
chrome.kill();
await sleep(300);
try {
  rmSync(PROFILE, { recursive: true, force: true });
} catch {}

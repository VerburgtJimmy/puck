/**
 * Layout assertions across widths: the install command must never overflow its
 * pill, the hero field and the river tail below the fold must stay locked
 * together, and nothing may push the page sideways.
 *
 *   node tools/check.mjs <url>
 */
import { spawn } from 'node:child_process';
import { rmSync } from 'node:fs';
import { setTimeout as sleep } from 'node:timers/promises';

const url = process.argv[2] ?? 'http://127.0.0.1:4350/';
const WIDTHS = [390, 560, 620, 768, 960, 1024, 1280, 1440, 1920, 2560];
const PORT = 9334;
const PROFILE = '/tmp/puck-check-profile';

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
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) {
    pending.get(m.id)(m.result);
    pending.delete(m.id);
  }
});
const send = (method, params = {}) =>
  new Promise((res) => {
    const n = ++id;
    pending.set(n, res);
    ws.send(JSON.stringify({ id: n, method, params }));
  });

const PROBE = `(() => {
  const vw = document.documentElement.clientWidth;
  const code = document.querySelector('.cmd code');
  const scene = document.querySelector('.hero__scene img').getBoundingClientRect();
  const hero = document.querySelector('.hero').getBoundingClientRect();
  const river = document.querySelector('.river img');
  const rr = river ? river.getBoundingClientRect() : null;
  const riderShown = river ? getComputedStyle(river).display !== 'none'
    && getComputedStyle(river.closest('.river')).display !== 'none' : false;
  return {
    vw,
    docW: document.documentElement.scrollWidth,
    codeOverflow: code.scrollWidth - code.clientWidth,
    codeFont: getComputedStyle(code).fontSize,
    sceneRightGap: Math.round(vw - scene.right),
    sceneBottomVsHero: Math.round(scene.bottom - hero.bottom),
    riverShown: riderShown,
    riverDx: rr ? Math.round(rr.left - scene.left) : null,
    riverDw: rr ? Math.round(rr.width - scene.width) : null,
  };
})()`;

let failures = 0;
for (const w of WIDTHS) {
  await send('Emulation.setDeviceMetricsOverride', { width: w, height: 900, deviceScaleFactor: 1, mobile: false });
  await send('Page.navigate', { url });
  await sleep(1400);
  const r = (await send('Runtime.evaluate', { expression: PROBE, returnByValue: true })).result.value;
  const bad = [];
  if (r.codeOverflow > 0) bad.push(`command overflows by ${r.codeOverflow}px`);
  if (r.docW > r.vw) bad.push(`page scrolls sideways (${r.docW} > ${r.vw})`);
  if (r.riverShown && (r.riverDx !== 0 || r.riverDw !== 0)) bad.push(`river offset dx=${r.riverDx} dw=${r.riverDw}`);
  if (r.riverShown && r.sceneBottomVsHero !== 0) bad.push(`scene bottom ${r.sceneBottomVsHero}px off the fold`);
  failures += bad.length ? 1 : 0;
  console.log(
    `${String(w).padStart(4)}  ${bad.length ? 'FAIL' : 'ok  '}  ` +
      `font ${r.codeFont}  right gap ${r.sceneRightGap}px  ` +
      `river ${r.riverShown ? 'joined' : 'hidden'}` +
      (bad.length ? `  <- ${bad.join('; ')}` : ''),
  );
}

ws.close();
chrome.kill();
await sleep(300);
try {
  rmSync(PROFILE, { recursive: true, force: true });
} catch {}
process.exit(failures ? 1 : 0);

const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

function copyText(text: string, btn?: HTMLButtonElement | null) {
  navigator.clipboard.writeText(text).then(() => {
    if (!btn) return;
    const prev = btn.textContent;
    btn.textContent = 'Copied';
    setTimeout(() => {
      btn.textContent = prev;
    }, 1200);
  });
}

document.querySelectorAll<HTMLElement>('[data-copy]').forEach((el) => {
  const btn = el.querySelector<HTMLButtonElement>('[data-copy-btn]');
  const code = el.querySelector('code')?.textContent?.trim() ?? '';
  btn?.addEventListener('click', () => copyText(code, btn));
});

document.querySelectorAll<HTMLButtonElement>('[data-copy-text]').forEach((btn) => {
  btn.addEventListener('click', () => copyText(btn.dataset.copyText ?? '', btn));
});

/* Parallax */
const layers = document.querySelectorAll<HTMLElement>('[data-parallax]');
if (!reduce && layers.length) {
  let ticking = false;
  const update = () => {
    const y = window.scrollY;
    layers.forEach((layer) => {
      const factor = Number(layer.dataset.parallax || 0);
      layer.style.transform = `translate3d(0, ${y * factor}px, 0)`;
    });
    ticking = false;
  };
  window.addEventListener(
    'scroll',
    () => {
      if (!ticking) {
        ticking = true;
        requestAnimationFrame(update);
      }
    },
    { passive: true },
  );
  update();
}

/* Bench bars */
const barBlocks = document.querySelectorAll<HTMLElement>('[data-bars]');
if (barBlocks.length) {
  const io = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (!entry.isIntersecting) return;
        entry.target.querySelectorAll<HTMLElement>('[data-bar]').forEach((bar) => {
          if (reduce) bar.style.width = getComputedStyle(bar).getPropertyValue('--w');
          else bar.classList.add('is-on');
        });
        io.unobserve(entry.target);
      });
    },
    { threshold: 0.4 },
  );
  barBlocks.forEach((el) => io.observe(el));
}

/* How it works */
const how = document.querySelector<HTMLElement>('[data-how]');
if (how) {
  if (reduce) how.classList.add('is-on');
  else {
    const io = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (entry.isIntersecting) {
            how.classList.add('is-on');
            io.disconnect();
          }
        });
      },
      { threshold: 0.35 },
    );
    io.observe(how);
  }
}

/* Install tabs */
const install = document.querySelector('[data-install]');
if (install) {
  const tabs = install.querySelectorAll<HTMLButtonElement>('[data-install-tab]');
  const panes = install.querySelectorAll<HTMLElement>('[data-install-pane]');
  tabs.forEach((tab) => {
    tab.addEventListener('click', () => {
      const id = tab.dataset.installTab;
      tabs.forEach((t) => {
        const on = t === tab;
        t.classList.toggle('is-active', on);
        t.setAttribute('aria-selected', String(on));
      });
      panes.forEach((pane) => {
        const on = pane.dataset.installPane === id;
        pane.classList.toggle('is-active', on);
        pane.hidden = !on;
      });
    });
  });
}

/* Terminal replay */
type Frames = [number, string][];
const frameEl = document.getElementById('term-frames');
const termRoot = document.querySelector('[data-term]');
if (frameEl && termRoot) {
  const data = JSON.parse(frameEl.textContent || '{}') as {
    puck: Frames;
    composer: Frames;
  };
  const out = termRoot.querySelector<HTMLElement>('[data-term-out]');
  const tabs = termRoot.querySelectorAll<HTMLButtonElement>('[data-term-tab]');
  let timer: number | undefined;
  let gen = 0;

  const play = (key: 'puck' | 'composer') => {
    if (!out) return;
    window.clearTimeout(timer);
    const my = ++gen;
    const frames = data[key] ?? [];
    out.textContent = '';
    if (reduce) {
      out.textContent = frames.map(([, line]) => line).join('\n');
      return;
    }
    let i = 0;
    let buf = '';
    const step = () => {
      if (my !== gen) return;
      if (i >= frames.length) return;
      const [delay, line] = frames[i++];
      timer = window.setTimeout(() => {
        buf += (buf ? '\n' : '') + line;
        out.textContent = buf;
        step();
      }, delay);
    };
    step();
  };

  tabs.forEach((tab) => {
    tab.addEventListener('click', () => {
      const key = (tab.dataset.termTab || 'puck') as 'puck' | 'composer';
      tabs.forEach((t) => {
        const on = t === tab;
        t.classList.toggle('is-active', on);
        t.setAttribute('aria-selected', String(on));
      });
      play(key);
    });
  });

  const io = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (entry.isIntersecting) {
          play('puck');
          io.disconnect();
        }
      });
    },
    { threshold: 0.35 },
  );
  io.observe(termRoot);
}

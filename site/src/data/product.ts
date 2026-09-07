export const BENCH_RUN =
  'https://github.com/VerburgtJimmy/puck/actions/runs/34124132162';

export const BENCHES = {
  skeleton: {
    title: 'laravel-skeleton --no-dev',
    note: 'Linux CI. Warm-wipe is the honest CI number.',
    rows: [
      { scenario: 'warm wipe', composer: 1662, puck: 327 },
      { scenario: 'warm keep', composer: 908, puck: 24 },
      { scenario: 'cold', composer: 4094, puck: 2029 },
    ],
  },
  app: {
    title: 'laravel-app with require-dev',
    note: 'Warm-wipe time is dominated by the optimized classmap dump.',
    rows: [
      { scenario: 'warm wipe', composer: 2814, puck: 504 },
      { scenario: 'warm keep', composer: 1380, puck: 25 },
      { scenario: 'cold', composer: 4422, puck: 1778 },
    ],
  },
} as const;

export const COMPAT = [
  { area: 'composer.json / lock', status: 'Supported' },
  { area: 'Composer + path repos', status: 'Supported' },
  { area: 'install / require / remove / update', status: 'Supported' },
  { area: 'Pest + phpstan Tier 1 adapters', status: 'Supported' },
  { area: 'vcs / artifact / package repos', status: 'Not in 0.1' },
  { area: 'Windows', status: 'Not in 0.1' },
  { area: 'Broader plugin host', status: 'Not in 0.1' },
  { area: 'Replace Composer entirely', status: 'Not planned' },
] as const;

/** Frames: [delay_ms before this line appears, line text] */
export const PUCK_INSTALL_FRAMES: [number, string][] = [
  [0, '$ puck install'],
  [80, 'puck: plan  install=112  update=0  keep=0  remove=0'],
  [40, 'puck: downloading laravel/framework'],
  [30, 'puck: downloading nesbot/carbon'],
  [25, 'puck: downloading symfony/http-foundation'],
  [20, 'puck: downloading pestphp/pest'],
  [40, 'puck: installed laravel/framework'],
  [20, 'puck: installed nesbot/carbon'],
  [15, 'puck: installed symfony/http-foundation'],
  [15, 'puck: installed pestphp/pest'],
  [80, 'puck: dumped autoload (-o)'],
  [40, 'puck: discovered 14 packages'],
  [30, 'puck: pest plugins dumped (3)'],
  [20, 'puck: done'],
  [60, 'puck: timing  total_ms=1778'],
];

export const COMPOSER_INSTALL_FRAMES: [number, string][] = [
  [0, '$ composer install'],
  [200, 'Installing dependencies from lock file'],
  [400, 'Package operations: 112 installs'],
  [350, '  - Installing laravel/framework'],
  [380, '  - Installing nesbot/carbon'],
  [360, '  - Installing symfony/http-foundation'],
  [340, '  - Installing pestphp/pest'],
  [500, 'Generating optimized autoload files'],
  [420, ''],
  [200, '112 packages installed in 4.42s'],
];

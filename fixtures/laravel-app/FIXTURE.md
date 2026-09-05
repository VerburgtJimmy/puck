# laravel-app fixture

Laravel skeleton-based app plus Tier 1 plugin surface for M3:

- `pestphp/pest` + `pestphp/pest-plugin` (composer-plugin)
- `larastan/larastan` + `phpstan/extension-installer` (composer-plugin)

`laravel/pao` was removed from require-dev (conflicts with Pest on Laravel 13).

App tree from `laravel/laravel` `v13.10.1`; framework lock still tracks the skeleton pin.

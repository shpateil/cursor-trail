# cursor-trail (форк)

след за курсором для hyprland с темами и тремя стилями отрисовки.
основа — seekator/cursor-trail (MIT), переделан рендер и добавлено управление.

## отличия от оригинала

- три стиля: `brush` (сужающийся мазок), `arrow` (шевроны как в osu), `particle` (ромбики/частицы)
- сужение к хвосту (`--taper`) вместо постоянной толщины
- градиент двух цветов (`--head` / `--tail`) вместо одного оттенка
- свечение (`--glow`)
- конфиг `~/.config/cursor-trail/config.toml`
- `--dump-png` — отрендерить след в файл для проверки без скриншотов

## управление

```
cursor-trail-theme on | off | toggle | status
cursor-trail-theme theme <имя>     сменить тему
cursor-trail-theme list            список тем
cursor-trail-theme set --style arrow --head '#ff5fd9' --glow 0.6
cursor-trail-theme preview <имя>   посмотреть на 6 секунд
```

## сборка

```bash
export CARGO_HOME=/media/raid0/tmp/cargo-home
cargo build --release --offline
```

## лицензия

MIT, оригинал © seekator

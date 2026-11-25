# cursor-trail

след за курсором для hyprland (wayland, wlr-layer-shell). rust + tiny-skia,
без gpu, cpu в простое около нуля (кадры рисуются только пока след жив).

## визуал

- лента переменной ширины: ширина и жизнь зависят от скорости движения
- catmull-rom сглаживание, усреднённые нормали на стыках (без разрывов)
- аддитивный ореол (plus blending), блик головы с лёгким пульсом
- цвет по OKLab от головы (#ff5fd9) к хвосту (#8b5cf6)
- три стиля: brush (лента), stamps (штампы), stars (частицы как в osu)
- скрывается на полный экран

## сборка

нужен rust stable и wayland dev-пакеты (libwayland, libxkbcommon).

```sh
cargo build --release
```

бинарник в `target/release/cursor-trail`. текстура звезды берётся из
`~/.local/share/cursor-trail/star.png`, если там нет, из `assets/star.png` рядом
с проектом.

## запуск

```sh
cursor-trail                 # с конфигом по умолчанию
cursor-trail --style stars   # другой стиль разово
cursor-trail --help          # все опции
```

как служба: systemd user unit с `After=graphical-session.target`, запускать в той
же сессии где hyprland (`WAYLAND_DISPLAY` в окружении). обёртка для управления и
готовыми темами лежит рядом: `cursor-trail-theme` (on/off/toggle/theme/set/status).

## конфиг

`~/.config/cursor-trail/config.toml`, ключи: `life`, `max_width`, `taper`, `style`,
`shape`, `color_head`, `color_tail`, `glow`, `thickness`, `smoothing`,
`activation_speed`, `alpha`, `density`, `spin`, `hide_on_fullscreen`. полный
список с дефолтами: `cursor-trail --help`.

пример:

```toml
style = "brush"
max_width = 9
life = 420
color_head = "#ff5fd9"
color_tail = "#8b5cf6"
glow = 0.55
activation_speed = 320
hide_on_fullscreen = true
```

## как это работает

слой wlr-layer-shell поверх всех окон, кадр рисуется только пока след живой, в
простое процесс не будит композитор. позиция курсора берётся из wayland pointer
events, поэтому на x11 и на не-wlroots композиторах не заведётся.

## лицензия

MIT
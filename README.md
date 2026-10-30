# cursor-trail

след за курсором для hyprland (wayland, wlr-layer-shell). rust + tiny-skia,
без gpu, cpu в простое около нуля (кадры рисуются только пока след жив).

## визуал
- лента переменной ширины: ширина и жизнь зависят от скорости движения
- catmull-rom сглаживание, усреднённые нормали на стыках (без разрывов)
- аддитивный ореол (plus blending), блик головы с лёгким пульсом
- цвет по OKLab от головы (#ff5fd9) к хвосту (#8b5cf6)
- три стиля: brush (лента), stamps (штампы), stars (частицы как в osu)

## запуск
systemd user unit: `~/.config/systemd/user/cursor-trail.service`,
управление через `cursor-trail-theme` (on/off/toggle/theme/set/status).

автостарт: unit в graphical-session.target, поднимается `cursor-trail-theme resume`
из autostart.lua hyprland.

## конфиг
~/.config/cursor-trail/config.toml, ключи: life, max_width, taper, style, shape,
color_head, color_tail, glow, thickness, smoothing, activation_speed, alpha,
density, spin, hide_on_fullscreen и др. полный список: `cursor-trail --help`.

## сборка
cargo build --release

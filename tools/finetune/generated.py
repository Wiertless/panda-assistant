"""Generated families of training conversations: common requests over pools of
real values and several wordings per language, with the right call and a short
answer. Written by Claude (2026-11-24); every value is spelled out (Polish forms
too), nothing is inflected by rule. data.py imports this and checks it all.
"""

import random

from data_dsl import T, chat, ex

R = random.Random(7)


def pick(xs, k):
    return R.sample(xs, min(k, len(xs)))


def plural(n, one, few, many):
    """Polish noun form for a number: 1 minuta, 2-4 minuty (not 12-14), 5+ minut."""
    if n == 1:
        return one
    if n % 10 in (2, 3, 4) and n % 100 not in (12, 13, 14):
        return few
    return many


assert [plural(n, "minuta", "minuty", "minut") for n in (1, 2, 5, 12, 22, 25)] == \
    ["minuta", "minuty", "minut", "minut", "minuty", "minut"]


# ------------------------------------------------------------ on/off settings
TOGGLES = [
    # tool, arg, on result, off result, EN on, EN off, PL on, PL off, PL answer on, PL answer off
    ("set_dark_mode", "enabled", "Dark mode on.", "Dark mode off.",
     ["dark theme please", "enable dark mode", "go dark", "switch the desktop to dark"],
     ["light mode please", "turn dark mode off", "back to the light theme"],
     ["włącz ciemny tryb", "przełącz na ciemny wygląd", "daj ciemny motyw"],
     ["włącz jasny motyw", "wyłącz tryb ciemny", "wróć do jasnego wyglądu"],
     "Ciemny motyw włączony.", "Jasny motyw włączony."),
    ("set_night_light", "enabled", "Night Light on.", "Night Light off.",
     ["turn on night light", "make the screen warmer for the evening", "enable the blue light filter"],
     ["night light off", "turn off the warm screen", "disable the blue light filter"],
     ["włącz światło nocne", "włącz filtr niebieskiego światła", "ociepl kolory ekranu"],
     ["wyłącz filtr niebieskiego światła", "wyłącz światło nocne"],
     "Światło nocne włączone.", "Światło nocne wyłączone."),
    ("set_do_not_disturb", "enabled", "Do Not Disturb on.", "Do Not Disturb off.",
     ["silence notifications", "turn on do not disturb", "no notifications for now"],
     ["notifications back on", "turn off do not disturb", "I want notifications again"],
     ["wycisz powiadomienia", "bez powiadomień przez chwilę", "nie przeszkadzać, proszę"],
     ["włącz z powrotem powiadomienia", "wyłącz nie przeszkadzać"],
     "Powiadomienia wyciszone.", "Powiadomienia znowu włączone."),
    ("set_wifi", "enabled", "Wi-Fi on.", "Wi-Fi off.",
     ["wifi on", "turn the wifi back on", "enable wireless"],
     ["switch wifi off", "disable wifi for now", "wifi off please"],
     ["włącz wifi", "włącz internet bezprzewodowy"],
     ["wyłącz wifi", "odłącz wi-fi na razie"],
     "Wi-Fi włączone.", "Wi-Fi wyłączone."),
    ("set_bluetooth", "enabled", "Bluetooth on.", "Bluetooth off.",
     ["bluetooth on", "turn on bluetooth please", "enable bluetooth"],
     ["bluetooth off", "disable bluetooth", "switch bluetooth off to save battery"],
     ["włącz bluetooth proszę", "uruchom bluetooth"],
     ["wyłącz bluetooth", "wyłącz bluetootha, oszczędzam baterię"],
     "Bluetooth włączony.", "Bluetooth wyłączony."),
]
for tool, arg, on_r, off_r, en_on, en_off, pl_on, pl_off, pl_a_on, pl_a_off in TOGGLES:
    for u in en_on:
        ex(u, T(tool, on_r, **{arg: True}))
    for u in en_off:
        ex(u, T(tool, off_r, **{arg: False}))
    for u in pl_on:
        ex(u, T(tool, on_r, **{arg: True}), answer=pl_a_on)
    for u in pl_off:
        ex(u, T(tool, off_r, **{arg: False}), answer=pl_a_off)

for u, muted, ans in [("mute the sound", True, None), ("unmute", False, None), ("sound back on", False, None),
                      ("wycisz komputer", True, "Dźwięk wyciszony."), ("wyłącz wyciszenie", False, "Dźwięk włączony.")]:
    ex(u, T("set_mute", "Sound muted." if muted else "Sound unmuted.", muted=muted), answer=ans)

# ------------------------------------------------------------ volume and brightness
for pct in pick([5, 10, 15, 20, 25, 30, 35, 40, 60, 65, 70, 80, 85, 90, 100], 9):
    w = R.choice(["volume to {p}", "set volume {p}%", "put the volume at {p} percent", "volume {p}"])
    ex(w.format(p=pct), T("set_volume", f"Volume set to {pct}%.", percent=pct))
    w = R.choice(["głośność na {p}", "ustaw głośność {p}%", "daj głośność {p} procent"])
    ex(w.format(p=pct), T("set_volume", f"Volume set to {pct}%.", percent=pct), answer=f"Głośność ustawiona na {pct}%.")
for u in ["louder", "a bit louder please", "turn it up", "I can't hear it, louder"]:
    ex(u, T("set_volume", f"Volume set to {R.choice([55, 60, 65, 70, 75])}%.", change="up"))
for u in ["quieter", "turn it down please", "too loud", "a little quieter"]:
    ex(u, T("set_volume", f"Volume set to {R.choice([25, 30, 35, 40, 45])}%.", change="down"))
for u, ch in [("głośniej proszę", "up"), ("podgłośnij trochę", "up"), ("za cicho", "up"),
              ("ciszej", "down"), ("za głośno", "down"), ("ścisz muzykę", "down")]:
    v = R.choice([60, 65, 70]) if ch == "up" else R.choice([30, 35, 40])
    ex(u, T("set_volume", f"Volume set to {v}%.", change=ch),
       answer=f"{'Podgłośniłem' if ch == 'up' else 'Ściszyłem'} do {v}%.")
for u, ch in [("brighter", "up"), ("make the screen brighter", "up"), ("dim the screen", "down"),
              ("screen is too bright", "down")]:
    v = R.choice([70, 80, 90]) if ch == "up" else R.choice([20, 30, 40])
    ex(u, T("set_brightness", f"Brightness set to {v}%.", change=ch))
for u, ch in [("rozjaśnij ekran", "up"), ("jaśniej", "up"), ("przyciemnij ekran", "down"), ("za jasno", "down")]:
    v = R.choice([70, 80, 90]) if ch == "up" else R.choice([20, 30, 40])
    ex(u, T("set_brightness", f"Brightness set to {v}%.", change=ch),
       answer=f"{'Rozjaśniłem' if ch == 'up' else 'Przyciemniłem'} ekran do {v}%.")
for pct in [25, 50, 75, 100]:
    ex(f"brightness {pct}%", T("set_brightness", f"Brightness set to {pct}%.", percent=pct))

# ------------------------------------------------------------ apps
APPS = [("Firefox", "Firefox"), ("Files", "Files"), ("Text Editor", "Text Editor"), ("Spotify", "Spotify"),
        ("GIMP", "GIMP"), ("Steam", "Steam"), ("Calendar", "Calendar"), ("Ptyxis", "Terminal"),
        ("Loupe", "Image Viewer"), ("Gapless", "Gapless"), ("LibreOffice Writer", "LibreOffice Writer"),
        ("Zed", "Zed"), ("Convey", "Convey"), ("Discord", "Discord")]
for name, shown in pick(APPS, 12):
    w = R.choice(["open {a}", "launch {a}", "start {a}", "can you open {a}"])
    ex(w.format(a=name), T("open_app", f"Opened {shown}.", name=name))
for name, shown in pick(APPS, 8):
    w = R.choice(["otwórz {a}", "uruchom {a}", "odpal {a}"])
    ex(w.format(a=name), T("open_app", f"Opened {shown}.", name=name), answer=f"Otworzyłem {shown}.")
for kind, shown in [("browser", "Firefox"), ("music", "Gapless"), ("files", "Files"), ("terminal", "Ptyxis")]:
    ex(f"open my {kind}", T("open_app", f"Opened {shown}.", name=kind))
# an app that isn't there: look it up, offer what is
ex("open photoshop",
   T("open_app", "Error: no app called photoshop. Close names: GIMP, Pinta.", name="photoshop"),
   answer="Photoshop isn't installed. GIMP or Pinta can edit pictures; want me to open one?")
ex("otwórz worda",
   T("open_app", "Error: no app called word. Close names: LibreOffice Writer.", name="word"),
   T("open_app", "Opened LibreOffice Writer.", name="LibreOffice Writer"),
   answer="Worda nie ma; otworzyłem LibreOffice Writer.")

# ------------------------------------------------------------ sums
def fmt(x):
    s = f"{x:.12f}".rstrip("0").rstrip(".")
    return s if s != "-0" else "0"


SUMS = []
for _ in range(14):
    a, b = R.randint(12, 980), R.randint(2, 97)
    op = R.choice(["+", "-", "*", "/"])
    v = {"+": a + b, "-": a - b, "*": a * b, "/": a / b}[op]
    SUMS.append((f"{a} {op} {b}", v))
for i, (expr, v) in enumerate(SUMS):
    en = R.choice(["what's {e}", "calculate {e}", "how much is {e}", "{e} = ?"]).format(e=expr.replace("*", "times").replace("/", "divided by"))
    pl = R.choice(["ile to {e}", "policz {e}", "oblicz {e}"]).format(e=expr.replace("*", "razy").replace("/", "podzielić przez"))
    r = f"{expr} = {fmt(v)}"
    shown = fmt(round(v, 2))
    if i % 2:
        ex(en, T("calculate", r, expression=expr), answer=f"{expr.replace('*', '×').replace('/', '÷')} = {shown}")
    else:
        ex(pl, T("calculate", r, expression=expr), answer=f"To {shown.replace('.', ',')}.")
for pct, base in [(15, 80), (20, 349), (7, 1200), (12.5, 64), (30, 45.9), (19, 2350)]:
    expr = f"{fmt(pct)}% * {fmt(base)}"
    v = pct / 100 * base
    ex(R.choice(["what's {p}% of {b}", "{p} percent of {b}", "how much is {p}% of {b}"]).format(p=fmt(pct), b=fmt(base)),
       T("calculate", f"{expr} = {fmt(v)}", expression=expr), answer=fmt(round(v, 2)))
    ex(R.choice(["ile to {p} procent z {b}", "ile to {p}% z {b}", "policz {p} proc z {b}"]).format(p=fmt(pct).replace(".", ","), b=fmt(base).replace(".", ",")),
       T("calculate", f"{expr} = {fmt(v)}", expression=expr), answer=f"To {fmt(round(v, 2)).replace('.', ',')}.")
for total, people in [(240, 3), (97.5, 2), (1380, 6)]:
    expr = f"{fmt(total)} / {people}"
    v = total / people
    ex(f"we spent {fmt(total)} zł, split between {people}", T("calculate", f"{expr} = {fmt(v)}", expression=expr),
       answer=f"{fmt(round(v, 2))} zł each.")
    ex(f"podziel {fmt(total).replace('.', ',')} zł na {people} {plural(people, 'osobę', 'osoby', 'osób')}",
       T("calculate", f"{expr} = {fmt(v)}", expression=expr),
       answer=f"Po {fmt(round(v, 2)).replace('.', ',')} zł na osobę.")

# ------------------------------------------------------------ timers and reminders
for m in pick([2, 3, 5, 7, 12, 15, 20, 25, 40, 45, 90], 7):
    ex(R.choice(["set a timer for {m} minutes", "{m} minute timer", "timer {m} min"]).format(m=m),
       T("set_reminder", f"Timer set for {m} minutes.", in_minutes=m))
    mins = plural(m, "minutę", "minuty", "minut")
    ex(R.choice(["nastaw minutnik na {m} {w}", "timer na {m} {w}", "odlicz {m} {w}"]).format(m=m, w=mins),
       T("set_reminder", f"Timer set for {m} minutes.", in_minutes=m), answer=f"Minutnik na {m} {mins} nastawiony.")
for what_en, what_pl, m in [("take the laundry out", "wyjąć pranie", 50), ("call mum", "zadzwonić do mamy", 120),
                            ("check the bus", "sprawdzić autobus", 15), ("stretch", "rozciągnąć się", 60)]:
    ex(f"remind me in {m} minutes to {what_en}",
       T("set_reminder", f"I'll remind you in {m} minutes: {what_en}.", in_minutes=m, text=what_en))
    mins = plural(m, "minutę", "minuty", "minut")
    ex(f"przypomnij mi za {m} {mins}, żeby {what_pl}",
       T("set_reminder", f"I'll remind you in {m} minutes: {what_pl}.", in_minutes=m, text=what_pl),
       answer=f"Przypomnę ci za {m} {mins}.")

# ------------------------------------------------------------ weather
CITIES = [("Toruń", "w Toruniu"), ("Warszawa", "w Warszawie"), ("Wrocław", "we Wrocławiu"), ("Poznań", "w Poznaniu"),
          ("Łódź", "w Łodzi"), ("Szczecin", "w Szczecinie"), ("Lublin", "w Lublinie"), ("Zakopane", "w Zakopanem"),
          ("Berlin", "w Berlinie"), ("Lisbon", "w Lizbonie"), ("London", "w Londynie"), ("Prague", "w Pradze")]
SKY = [("clear", "bezchmurnie"), ("cloudy", "pochmurno"), ("light rain", "lekki deszcz"), ("snow", "śnieg"),
       ("fog", "mgła"), ("sunny", "słonecznie")]
for city, loc in CITIES:
    t = R.randint(-4, 16)
    sky_en, sky_pl = R.choice(SKY)
    res = f"{city} now: {t}°C, {sky_en}, wind {R.randint(5, 30)} km/h.\nTomorrow: {t - 2}–{t + 3}°C, {R.choice(SKY)[0]}."
    if R.random() < 0.5:
        ex(R.choice(["weather in {c}", "what's the weather like in {c}", "how cold is it in {c}"]).format(c=city),
           T("get_weather", res, place=city), answer=f"{t}°C and {sky_en} in {city}.")
    else:
        ex(R.choice(["jaka pogoda {l}?", "jak jest {l}?", "ile stopni {l}?"]).format(l=loc),
           T("get_weather", res, place=city), answer=f"{loc.capitalize()} {t}°C, {sky_pl}.")

# ------------------------------------------------------------ files: find, then act
FILES = [("cv", "~/Dokumenty/CV Jakub 2026.pdf", "CV Jakub 2026.pdf"),
         ("invoice", "~/Pobrane/faktura-11.pdf", "faktura-11.pdf"),
         ("boarding pass", "~/Pobrane/boarding pass LIS.pdf", "boarding pass LIS.pdf"),
         ("thesis", "~/Dokumenty/praca/praca magisterska v7.docx", "praca magisterska v7.docx"),
         ("budget", "~/Dokumenty/budżet 2026.ods", "budżet 2026.ods"),
         ("lease", "~/Dokumenty/umowa najmu 2026.pdf", "umowa najmu 2026.pdf")]
for key, path, name in FILES:
    found = f"{path} ({R.randint(80, 2400)} KB, 2026-{R.randint(1, 11):02d}-{R.randint(1, 28):02d})"
    ex(R.choice(["send my {k} to my phone", "put my {k} on my iphone", "I need my {k} on my phone"]).format(k=key),
       T("find_files", found, name=key),
       T("send_file", f"Sent {name} to jakubs-iphone with Taildrop.", device="jakubs-iphone", path=path),
       answer=f"Sent {name} to your iPhone.")
    ex(R.choice(["open my {k}", "where's my {k}", "find my {k}"]).format(k=key),
       T("find_files", found, name=key),
       *([T("open_file", f"Opened {name} in its app.", path=path)] if R.random() < 0.5 else []),
       answer=f"{name}: {path}.")
ex("wyślij mi CV na telefon",
   T("find_files", "~/Dokumenty/CV Jakub 2026.pdf (310 KB, 2026-10-02)", name="CV"),
   T("send_file", "Sent CV Jakub 2026.pdf to jakubs-iphone with Taildrop.", device="jakubs-iphone", path="~/Dokumenty/CV Jakub 2026.pdf"),
   answer="Wysłałem CV na iPhone'a.")
ex("otwórz moją pracę magisterską",
   T("find_files", "~/Dokumenty/praca/praca magisterska v7.docx (1.4 MB, 2026-11-12)\n~/Dokumenty/praca/praca magisterska v6.docx (1.3 MB, 2026-10-30)", name="praca magisterska"),
   T("open_file", "Opened praca magisterska v7.docx in LibreOffice Writer.", path="~/Dokumenty/praca/praca magisterska v7.docx"),
   answer="Otworzyłem najnowszą wersję, v7.")
# nothing found under one name: try another
ex("find my passport scan",
   T("find_files", "No files named like “passport scan”. Similar: paszport.jpg (~/Obrazy/dokumenty).", name="passport scan"),
   T("find_files", "~/Obrazy/dokumenty/paszport.jpg (2.2 MB, 2025-08-19)", name="paszport"),
   answer="It's ~/Obrazy/dokumenty/paszport.jpg.")

# ------------------------------------------------------------ pictures
for folder, n in [("Obrazy/Wakacje 2026", 31), ("Obrazy/Ślub Ani", 120), ("Pobrane/zdjęcia z telefonu", 14), ("Obrazy/Kot", 8)]:
    ex(R.choice(["make the photos in {f} smaller", "resize everything in {f} for email", "shrink {f}"]).format(f=folder),
       T("edit_image", f"Saved {n} of {n} in ~/Obrazy/Edited (the originals are unchanged):\n~/Obrazy/Edited/IMG_0001.jpg: 3.8 MB 4032×3024 → 0.6 MB 1920×1440\n…",
         files=folder, resize="1920"),
       answer=f"Done: {n} smaller copies in Obrazy/Edited; the originals are untouched.")
    ex(R.choice(["zmniejsz zdjęcia z {f}", "zmniejsz rozmiar zdjęć w {f}"]).format(f=folder),
       T("edit_image", f"Saved {n} of {n} in ~/Obrazy/Edited (the originals are unchanged):\n~/Obrazy/Edited/IMG_0001.jpg: 3.8 MB 4032×3024 → 0.6 MB 1920×1440\n…",
         files=folder, resize="1920"),
       answer=f"Gotowe: {n} mniejszych kopii w Obrazy/Edited, oryginały bez zmian.")
for folder, name in [("Pobrane/skany", "skany"), ("Dokumenty/rachunki", "rachunki"), ("Obrazy/notatki z tablicy", "notatki")]:
    ex(f"put the pictures in {folder} into one pdf",
       T("images_to_pdf", f"Made ~/Obrazy/Edited/{name}.pdf from {R.randint(3, 12)} pictures (2.1 MB)", files=folder, name=name),
       answer=f"Made {name}.pdf in Obrazy/Edited.")
ex("convert the photos in Pobrane to webp",
   T("edit_image", "Saved 9 of 9 in ~/Obrazy/Edited (the originals are unchanged):\n~/Obrazy/Edited/IMG_2231.webp: 2.9 MB 4032×3024 → 0.9 MB 4032×3024\n…",
     files="Downloads/*.jpg", format="webp"),
   answer="Done: 9 WebP copies in Obrazy/Edited.")
ex("compress screenshot.png on the desktop",
   T("edit_image", "Saved 1 of 1 in ~/Obrazy/Edited (the originals are unchanged):\n~/Obrazy/Edited/screenshot.jpg: 1.9 MB 2880×1920 → 0.3 MB 2880×1920",
     files="Desktop/screenshot.png", format="jpg", quality=80),
   answer="Compressed: 1.9 MB → 0.3 MB, in Obrazy/Edited/screenshot.jpg.")

# ------------------------------------------------------------ general knowledge, no tools
FACTS = [
    ("how many legs does a spider have", "Eight."),
    ("what's the boiling point of water", "100 °C at sea level."),
    ("who wrote Pan Tadeusz", "Adam Mickiewicz."),
    ("what does RAM stand for", "Random-access memory: the computer's working memory."),
    ("kto napisał Lalkę?", "Bolesław Prus."),
    ("ile dni ma luty w roku przestępnym?", "29."),
    ("co to jest NPU?", "To procesor do zadań AI: uruchamia sieci neuronowe, na przykład mnie, szybciej i oszczędniej niż CPU."),
    ("what is a VPN", "A VPN sends your traffic through an encrypted tunnel to another server, hiding it from the local network."),
    ("jaka jest stolica Kanady?", "Ottawa."),
    ("how far is the moon", "About 384,000 km on average."),
    ("what's a good name for a cat", "Miso, Pixel or Tofu; or Pierogi, if it's round."),
    ("co to znaczy 'deadline'?", "Ostateczny termin, do którego coś trzeba zrobić."),
    ("good morning", "Good morning! What's first today?"),
    ("dzień dobry!", "Dzień dobry! W czym mogę pomóc?"),
    ("you're great", "Thanks! Anything else?"),
    ("super, dziękuję", "Nie ma za co!"),
    ("who are you", "I'm Panda, an assistant running on your laptop: I can change settings, find and send files, check mail and the calendar, and more."),
    ("kim jesteś?", "Jestem Panda, asystent działający lokalnie na twoim laptopie."),
]
for q, a in FACTS:
    chat(q, a)

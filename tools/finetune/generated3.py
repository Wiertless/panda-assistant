"""Generated chains, recoveries and wordings (Claude, 2026-11-24): several tools in a
row where each needs the one before, mistakes the tools report and the model
fixes, and many ways of saying the everyday requests in both languages.
"""

import random

from data_dsl import T, ex
from generated import plural

R = random.Random(23)

PHONE = "jakubs-iphone"
FOLDERS = [("Obrazy/Wakacje 2026", 31), ("Obrazy/Kot", 8), ("Pobrane/zdjęcia z telefonu", 14), ("Obrazy/Ślub Ani", 120),
           ("Obrazy/Gdańsk", 22), ("Obrazy/Paragony", 6)]


def shrunk(folder, n):
    return T("edit_image", f"Saved {n} of {n} in ~/Obrazy/Edited (the originals are unchanged):\n~/Obrazy/Edited/IMG_0001.jpg: 3.6 MB 4032×3024 → 0.6 MB 1920×1440\n…",
             files=folder, resize="1920")


# ------------------------------------------------------------ chains: find / change / send
for folder, n in FOLDERS:
    ex(R.choice(["shrink the photos in {f} and send them to my phone", "make {f} smaller and put them on my iphone"]).format(f=folder),
       shrunk(folder, n),
       T("send_file", f"Sent {n} files from ~/Obrazy/Edited to {PHONE} with Taildrop.", device=PHONE, path="~/Obrazy/Edited"),
       answer=f"Shrunk {n} photos and sent them to your iPhone.")
    ex(R.choice(["zmniejsz zdjęcia z {f} i wyślij je na telefon", "zrób mniejsze zdjęcia z {f} i prześlij na iPhone'a"]).format(f=folder),
       shrunk(folder, n),
       T("send_file", f"Sent {n} files from ~/Obrazy/Edited to {PHONE} with Taildrop.", device=PHONE, path="~/Obrazy/Edited"),
       answer=f"Zmniejszyłem {n} {plural(n, 'zdjęcie', 'zdjęcia', 'zdjęć')} i wysłałem je na iPhone'a.")
for folder, name in [("Pobrane/skany", "skany"), ("Obrazy/Paragony", "paragony"), ("Dokumenty/rachunki", "rachunki")]:
    ex(f"make a pdf from {folder} and send it to my phone",
       T("images_to_pdf", f"Made ~/Obrazy/Edited/{name}.pdf from {R.randint(3, 9)} pictures (1.8 MB)", files=folder, name=name),
       T("send_file", f"Sent {name}.pdf to {PHONE} with Taildrop.", device=PHONE, path=f"~/Obrazy/Edited/{name}.pdf"),
       answer=f"Made {name}.pdf and sent it to your iPhone.")
    ex(f"zrób pdf z {folder} i wyślij go na telefon",
       T("images_to_pdf", f"Made ~/Obrazy/Edited/{name}.pdf from {R.randint(3, 9)} pictures (1.8 MB)", files=folder, name=name),
       T("send_file", f"Sent {name}.pdf to {PHONE} with Taildrop.", device=PHONE, path=f"~/Obrazy/Edited/{name}.pdf"),
       answer=f"Zrobiłem {name}.pdf i wysłałem na iPhone'a.")
SHOTS = "~/Obrazy/Zrzuty ekranu/Screenshot From 2026-11-16 11-04-22.png (720 KB, 2026-11-16)\n~/Obrazy/Zrzuty ekranu/Screenshot From 2026-11-12 20-14-09.png (540 KB, 2026-11-12)"
for u, ans in [("send today's screenshot to my phone as a pdf", "Made a PDF of today's screenshot and sent it to your iPhone."),
               ("dzisiejszy zrzut ekranu wyślij na telefon jako pdf", "Zrobiłem PDF z dzisiejszego zrzutu i wysłałem na iPhone'a.")]:
    ex(u, T("find_files", SHOTS, name="Screenshot", folder="Obrazy"),
       T("images_to_pdf", "Made ~/Obrazy/Edited/screenshot.pdf from 1 pictures (210 KB)",
         files="~/Obrazy/Zrzuty ekranu/Screenshot From 2026-11-16 11-04-22.png", name="screenshot"),
       T("send_file", f"Sent screenshot.pdf to {PHONE} with Taildrop.", device=PHONE, path="~/Obrazy/Edited/screenshot.pdf"),
       answer=ans)
for u, ans in [("take a screenshot and send it to my phone", "Took a screenshot and sent it to your iPhone."),
               ("zrób zrzut ekranu i wyślij mi go", "Zrobiłem zrzut i wysłałem go na iPhone'a.")]:
    ex(u, T("save_screenshot", "Saved the screenshot as ~/Obrazy/Zrzuty ekranu/Screenshot From 2026-11-16 11-20-03.png. To send it, pass exactly that path to send_file."),
       T("send_file", f"Sent Screenshot From 2026-11-16 11-20-03.png to {PHONE} with Taildrop.", device=PHONE,
         path="~/Obrazy/Zrzuty ekranu/Screenshot From 2026-11-16 11-20-03.png"),
       answer=ans)

# ------------------------------------------------------------ chains: mail / calendar / reminders
for sender, subj, body, title, start, place in [
    ("Gabinet Uśmiech", "Przypomnienie o wizycie", "Wizyta we czwartek 19.11 o 9:30.", "Dentysta", "2026-11-19 09:30", None),
    ("Ola Kowalska", "Kino?", "Może w sobotę o 20:00 w Cinema City?", "Kino z Olą", "2026-11-21 20:00", "Cinema City"),
    ("Dziekanat", "Egzamin poprawkowy", "Egzamin z analizy 25.11 o 12:00, sala 204.", "Egzamin z analizy", "2026-11-25 12:00", "sala 204"),
]:
    res = f"Mail matching “{sender}”, newest first:\n[home:{R.randint(500, 599)}] {sender} — {subj} (Sunday 2026-11-15 18:30, the newest)\n  {body}"
    args = dict(title=title, start=start, **({"place": place} if place else {}))
    ex(f"add what {sender} wrote about to my calendar", T("search_mail", res, query=sender),
       T("create_event", f"Added “{title}” on {start} to Personal.", **args),
       answer=f"Added “{title}” ({start.split()[0][5:].replace('-', '.')} {start.split()[1]}) to your calendar.")
    ex(f"wpisz do kalendarza to, o czym pisał {sender}", T("search_mail", res, query=sender),
       T("create_event", f"Added “{title}” on {start} to Personal.", **args),
       answer=f"Dodałem „{title}” do kalendarza.")
for city, rainy in [("Toruń", True), ("Poznań", False), ("Gdańsk", True), ("Kraków", False)]:
    res = f"{city} now: 6°C, cloudy.\nTomorrow: 3–8°C, {'rain in the morning' if rainy else 'sunny'}."
    steps = [T("get_weather", res, place=city)]
    if rainy:
        steps.append(T("set_reminder", "I'll remind you on Monday at 07:30: take an umbrella.", at="2026-11-16 07:30", text="take an umbrella"))
    ex(f"if it's going to rain tomorrow in {city}, remind me at 7:30 to take an umbrella", *steps,
       answer="Rain is forecast, so I'll remind you at 7:30." if rainy else f"No rain expected in {city} tomorrow, so no reminder.")

# ------------------------------------------------------------ recovery: wrong names the tools correct
for wrong, right in [("iphone", PHONE), ("my phone", PHONE), ("telefon", PHONE), ("jakub iphone", PHONE)]:
    ex(f"send notes.txt to {wrong}",
       T("send_file", f"Error: no device called {wrong}. Devices: {PHONE} (iOS, online), framework (linux, this computer).", device=wrong, path="Documents/notes.txt"),
       T("send_file", f"Sent notes.txt to {PHONE} with Taildrop.", device=PHONE, path="Documents/notes.txt"),
       answer="Sent notes.txt to your iPhone.")
for wrong, right, shown in [("libre office", "LibreOffice Writer", "LibreOffice Writer"), ("gnome files", "Files", "Files"),
                            ("vscode", "Zed", "Zed"), ("spotfy", "Spotify", "Spotify")]:
    ex(f"open {wrong}",
       T("open_app", f"Error: no app called {wrong}. Close names: {right}.", name=wrong),
       T("open_app", f"Opened {shown}.", name=right),
       answer=f"Opened {shown}.")
for wrong, right in [("Sony headphones", "Sony WH-1000XM5"), ("airpods", "Jakub's AirPods Pro"), ("the mouse", "MX Master 3")]:
    ex(f"connect {wrong}",
       T("connect_bluetooth", f"Error: no paired device called {wrong}. Paired: Jakub's AirPods Pro, Sony WH-1000XM5, MX Master 3.", device=wrong),
       T("connect_bluetooth", f"Connected to {right}.", device=right))
for wrong, found in [("Documents/notatki.txt", "~/Dokumenty/notatki 2026.txt"), ("Downloads/invoice.pdf", "~/Pobrane/faktura-11.pdf")]:
    ex(f"open {wrong}",
       T("open_file", f"Error: {wrong} doesn't exist. In that folder: {found.rsplit('/', 1)[1]}.", path=wrong),
       T("open_file", f"Opened {found.rsplit('/', 1)[1]} in its app.", path=found),
       answer=f"Opened {found.rsplit('/', 1)[1]} (the closest match).")

# ------------------------------------------------------------ more ways of saying things
WORDINGS = [
    ("set_dark_mode", {"enabled": True}, "Dark mode on.",
     ["my eyes hurt, dark theme", "can you make the screen dark", "enable the dark style", "dark mode pls"],
     ["oczy mnie bolą, daj ciemny motyw", "przełącz na ciemno", "ciemny styl proszę"], "Ciemny motyw włączony."),
    ("set_volume", {"change": "down"}, "Volume set to 40%.",
     ["it's too loud", "lower the volume", "turn the sound down"], ["za głośno jest", "przycisz", "zmniejsz głośność"], "Ściszyłem do 40%."),
    ("set_volume", {"change": "up"}, "Volume set to 70%.",
     ["I can barely hear it", "raise the volume", "a little louder"], ["nic nie słyszę, głośniej", "zwiększ głośność", "trochę głośniej"], "Podgłośniłem do 70%."),
    ("get_battery", {}, "Battery 41%, discharging, about 2 h 10 min left.",
     ["battery?", "how long will my battery last", "am I low on battery"], ["ile zostało baterii", "starczy mi baterii na 2 godziny?", "jaki poziom baterii?"], "41%, zostało około 2 godzin."),
    ("get_datetime", {}, "Monday 2026-11-16 14:05 CET",
     ["what time is it now", "date and time?", "is it monday today"], ["jaki dziś dzień?", "która jest godzina", "jaka dzisiaj data?"], "Poniedziałek, 16 listopada, 14:05."),
    ("set_reminder", {"in_minutes": 5}, "Timer set for 5 minutes.",
     ["5 min timer", "time 5 minutes for me", "tell me in 5 minutes"], ["minutnik 5 minut", "nastaw 5 minut", "daj znać za 5 minut"], "Minutnik na 5 minut."),
    ("list_reminders", {}, "1. 18:00 today: call mum",
     ["any reminders?", "what am I supposed to remember today"], ["jakie mam przypomnienia?", "o czym mam dziś pamiętać?"], "Jedno: o 18:00 telefon do mamy."),
]
for tool, args, res, en, pl, pl_ans in WORDINGS:
    en_ans = {"get_battery": "41%, about 2 h 10 min left.", "get_datetime": "Monday 16 November, 14:05.",
              "list_reminders": "One: call mum at 18:00."}.get(tool)
    for u in en:
        ex(u, T(tool, res, **args), answer=en_ans)
    for u in pl:
        ex(u, T(tool, res, **args), answer=pl_ans)

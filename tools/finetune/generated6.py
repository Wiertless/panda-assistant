"""Breadth (Claude, 2026-11-24): GNOME settings found by name then changed,
reminders at given times and days, settings panels, and general knowledge
answered in words, in both languages.
"""

import random

from data_dsl import T, chat, ex

R = random.Random(53)

SETTINGS = [
    # schema, key, type, now, want, EN request, PL request, EN answer, PL answer, query
    ("org.gnome.desktop.interface", "clock-format", "string", "24h", "12h", "use a 12-hour clock", "zegar 12-godzinny",
     "The clock is 12-hour now.", "Zegar jest teraz 12-godzinny.", "clock format"),
    ("org.gnome.desktop.peripherals.touchpad", "natural-scroll", "boolean", "true", "false", "turn off natural scrolling on the touchpad",
     "wyłącz naturalne przewijanie na touchpadzie", "Natural scrolling is off.", "Naturalne przewijanie wyłączone.", "natural scroll"),
    ("org.gnome.desktop.peripherals.mouse", "speed", "double", "0.0", "0.4", "make the mouse faster", "przyspiesz myszkę",
     "Mouse speed is up (0.4).", "Myszka jest szybsza (0,4).", "mouse speed"),
    ("org.gnome.desktop.screensaver", "lock-delay", "uint32", "0", "300", "lock the screen 5 minutes after it goes blank",
     "blokuj ekran 5 minut po wygaszeniu", "The screen locks 5 minutes after it blanks.", "Ekran zablokuje się 5 minut po wygaszeniu.", "lock delay"),
    ("org.gnome.desktop.session", "idle-delay", "uint32", "300", "900", "blank the screen after 15 minutes instead of 5",
     "wygaszaj ekran po 15 minutach", "The screen now blanks after 15 minutes.", "Ekran wygasi się po 15 minutach.", "idle delay"),
    ("org.gnome.desktop.wm.preferences", "button-layout", "string", "appmenu:close", "appmenu:minimize,maximize,close",
     "add minimize and maximize buttons to windows", "dodaj przyciski minimalizacji i maksymalizacji do okien",
     "Windows have minimize and maximize buttons now.", "Okna mają teraz przyciski minimalizacji i maksymalizacji.", "window buttons"),
    ("org.gnome.desktop.interface", "show-battery-percentage", "boolean", "false", "true", "battery percentage in the top bar please",
     "procent baterii na górnym pasku", "The top bar shows the battery percentage now.", "Górny pasek pokazuje procent baterii.", "battery percentage"),
    ("org.gnome.mutter", "dynamic-workspaces", "boolean", "true", "false", "I want a fixed number of workspaces",
     "chcę stałą liczbę obszarów roboczych", "Workspaces are fixed now (no longer dynamic).", "Obszary robocze są teraz stałe.", "workspaces"),
    ("org.gnome.desktop.input-sources", "xkb-options", "as", "[]", "['caps:escape']", "make caps lock act as escape",
     "niech caps lock działa jak escape", "Caps Lock now works as Escape.", "Caps Lock działa teraz jak Escape.", "caps lock"),
]
for schema, key, typ, now, want, en, pl, en_a, pl_a, query in SETTINGS:
    found = f"{schema} {key} ({typ}, now {now}): {query}"
    for u, a in ((en, en_a), (pl, pl_a)):
        ex(u, T("find_settings", found, query=query),
           T("change_setting", f"{key} set to {want}.", schema=schema, key=key, value=want), answer=a)
    ex(f"what's {query} set to",
       T("find_settings", found, query=query), answer=f"{key} is {now}.")

# ------------------------------------------------------------ reminders at a time
TIMES = [("tomorrow at 8", "jutro o 8", "2026-11-16 08:00", "on Monday at 08:00", "jutro o 8:00"),
         ("at 6pm", "o 18", "2026-11-15 18:00", "today at 18:00", "o 18:00"),
         ("on friday at 9", "w piątek o 9", "2026-11-20 09:00", "on Friday at 09:00", "w piątek o 9:00"),
         ("on wednesday evening at 7", "w środę o 19", "2026-11-18 19:00", "on Wednesday at 19:00", "w środę o 19:00"),
         ("at noon", "w południe", "2026-11-15 12:00", "today at 12:00", "w południe")]
THINGS = [("pay the rent", "zapłacić czynsz"), ("call the plumber", "zadzwonić do hydraulika"), ("water the plants", "podlać kwiaty"),
          ("send the report", "wysłać raport"), ("buy a gift for Ania", "kupić prezent dla Ani")]
for (en_t, pl_t, at, en_when, pl_when), (en_w, pl_w) in zip(TIMES, THINGS):
    ex(f"remind me {en_t} to {en_w}", T("set_reminder", f"I'll remind you {en_when}: {en_w}.", at=at, text=en_w))
    ex(f"przypomnij mi {pl_t}, żeby {pl_w}", T("set_reminder", f"I'll remind you {en_when}: {pl_w}.", at=at, text=pl_w),
       answer=f"Przypomnę ci {pl_when}.")

# ------------------------------------------------------------ settings panels
for en, pl, panel in [("open bluetooth settings", "otwórz ustawienia bluetooth", "bluetooth"), ("open wifi settings", "ustawienia wifi", "wifi"),
                      ("power settings", "otwórz ustawienia zasilania", "power"), ("open the mouse settings", "ustawienia myszy i touchpada", "mouse"),
                      ("printer settings", "otwórz ustawienia drukarki", "printers"), ("open privacy settings", "ustawienia prywatności", "privacy")]:
    name = {"bluetooth": "Bluetooth", "wifi": "Wi-Fi", "power": "Power", "mouse": "Mouse & Touchpad", "printers": "Printers", "privacy": "Privacy"}[panel]
    ex(en, T("open_settings", f"Opened Settings → {name}.", panel=panel))
    ex(pl, T("open_settings", f"Opened Settings → {name}.", panel=panel), answer=f"Otworzyłem ustawienia: {name}.")

# ------------------------------------------------------------ general knowledge, in words
for q, a in [
    ("how many minutes are in a day", "1,440."),
    ("what's the speed of light", "About 300,000 km/s (299,792 km/s exactly)."),
    ("who painted the Mona Lisa", "Leonardo da Vinci."),
    ("what's the largest ocean", "The Pacific."),
    ("what does HTTP stand for", "HyperText Transfer Protocol."),
    ("what's the difference between Wi-Fi 6 and Wi-Fi 7", "Wi-Fi 7 adds 320 MHz channels and multi-link operation: faster and lower latency, if the router supports it too."),
    ("is a tomato a fruit", "Botanically yes, a berry; in the kitchen it's used as a vegetable."),
    ("why is the sea salty", "Rivers carry minerals from rocks into the sea, and when water evaporates the salt stays behind."),
    ("ile kontynentów jest na świecie?", "Siedem: Afryka, Ameryka Północna, Ameryka Południowa, Antarktyda, Australia, Azja i Europa."),
    ("kto był pierwszym królem Polski?", "Bolesław Chrobry, koronowany w 1025 roku."),
    ("jaka jest najdłuższa rzeka w Polsce?", "Wisła, około 1 022 km."),
    ("co to jest SSD?", "Dysk półprzewodnikowy: przechowuje dane w pamięci flash, bez ruchomych części, dlatego jest szybki i cichy."),
    ("czym się różni RAM od dysku?", "RAM to szybka pamięć robocza, czyszczona po wyłączeniu; dysk trzyma pliki na stałe."),
    ("dlaczego liście zmieniają kolor jesienią?", "Drzewa przestają wytwarzać chlorofil, więc widać żółte i czerwone barwniki, które były pod zielenią."),
    ("ile nóg ma pająk?", "Osiem."),
    ("co to jest Linux?", "Darmowy system operacyjny o otwartym kodzie; na nim działa twój Fedora Silverblue."),
]:
    chat(q, a)

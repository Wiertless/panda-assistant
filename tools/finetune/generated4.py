"""Generated documents questions and follow-ups (Claude, 2026-11-24): the user's
documents answered from the document itself (find_files, then ask_document,
with the answer naming what the document says), and second turns that lean on
the first ("a jutro?", "open it", "make it 20 minutes").
"""

import random

from data_dsl import T, ex

R = random.Random(31)

DOCS = [
    ("~/Dokumenty/umowa najmu 2026.pdf", "umowa najmu", [
        ("how much is the rent", "ile wynosi czynsz?", "§ 4. Czynsz najmu wynosi 2 400 zł miesięcznie, płatny do 10. dnia miesiąca.",
         "2,400 zł a month, due by the 10th.", "2 400 zł miesięcznie, płatne do 10. dnia miesiąca."),
        ("can I keep a cat", "czy mogę mieć kota?", "§ 11. Trzymanie zwierząt domowych wymaga pisemnej zgody Wynajmującego.",
         "Only with the landlord's written consent (§ 11).", "Tylko za pisemną zgodą wynajmującego (§ 11)."),
        ("when does the lease end", "do kiedy jest umowa?", "§ 2. Umowa zawarta na czas określony od 01.02.2026 do 31.01.2027.",
         "31 January 2027.", "Do 31 stycznia 2027."),
    ]),
    ("~/Dokumenty/praca/umowa o pracę 2025.pdf", "umowa o pracę", [
        ("how many vacation days do I get", "ile mam dni urlopu?", "§ 6. Pracownikowi przysługuje 26 dni urlopu wypoczynkowego w roku.",
         "26 days a year.", "26 dni w roku."),
        ("what are my working hours", "jakie mam godziny pracy?", "§ 3. Czas pracy: 8 godzin dziennie, od poniedziałku do piątku, 9:00–17:00.",
         "8 hours a day, Monday to Friday, 9:00–17:00.", "8 godzin dziennie, pon.–pt., 9:00–17:00."),
    ]),
    ("~/Dokumenty/polisa OC 2026.pdf", "polisa", [
        ("what number do I call after an accident", "pod jaki numer dzwonić po wypadku?", "Zgłoszenie szkody: infolinia 801 102 102 (24/7) lub aplikacja.",
         "801 102 102, any time (or the app).", "801 102 102, całą dobę (albo w aplikacji)."),
        ("does it cover the windscreen", "czy polisa obejmuje szybę?", "Zakres: OC posiadacza pojazdu. Ubezpieczenie szyb nie jest objęte polisą.",
         "No: it's third-party liability only, the windscreen isn't covered.", "Nie: to samo OC, szyby nie są objęte."),
    ]),
    ("~/Pobrane/instrukcja zmywarki Bosch.pdf", "instrukcja zmywarki", [
        ("what does error E15 mean on the dishwasher", "co znaczy błąd E15 w zmywarce?", "E15: zadziałał system ochrony przed wyciekiem (Aquastop). Odłącz zasilanie i sprawdź, czy w podstawie nie ma wody.",
         "The leak protection (Aquastop) tripped: unplug it and check for water in the base.", "Zadziałała ochrona przed wyciekiem (Aquastop): odłącz zasilanie i sprawdź, czy w podstawie nie ma wody."),
        ("how do I clean the dishwasher filter", "jak wyczyścić filtr w zmywarce?", "Filtr: przekręć w lewo, wyjmij, przepłucz pod bieżącą wodą, włóż i przekręć w prawo do oporu.",
         "Turn it left, take it out, rinse under running water, put it back and turn right until it stops.", "Przekręć w lewo, wyjmij, przepłucz pod wodą, włóż i przekręć w prawo do oporu."),
    ]),
    ("~/Dokumenty/praca/praca magisterska v7.docx", "praca magisterska", [
        ("what's the title of my thesis", "jaki jest tytuł mojej pracy?", "Tytuł: „Lokalne modele językowe na procesorach NPU: wydajność i jakość”.",
         "“Lokalne modele językowe na procesorach NPU: wydajność i jakość”.", "„Lokalne modele językowe na procesorach NPU: wydajność i jakość”."),
    ]),
]
for path, name, qas in DOCS:
    found = f"{path} ({R.randint(120, 2400)} KB, 2026-{R.randint(1, 11):02d}-{R.randint(1, 28):02d})"
    for q_en, q_pl, text, a_en, a_pl in qas:
        quote = f"From {path}: {text}"
        ex(q_en, T("find_files", found, name=name), T("ask_document", quote, path=path, question=q_en), answer=a_en)
        ex(q_pl, T("find_files", found, name=name), T("ask_document", quote, path=path, question=q_pl), answer=a_pl)

# ------------------------------------------------------------ follow-ups
FOLLOW = [
    (("jaka pogoda w Poznaniu?", "W Poznaniu 4°C, pochmurno."), "a jutro?",
     T("get_weather", "Poznań now: 4°C, cloudy.\nTomorrow: 2–7°C, light rain.", place="Poznań"), "Jutro 2–7°C i lekki deszcz."),
    (("weather in London", "9°C and light rain in London."), "and the day after?",
     T("get_weather", "London now: 9°C, light rain.\nTomorrow: 8–12°C, cloudy.\nWednesday: 7–11°C, sunny.", place="London"), "Wednesday looks sunny, 7–11°C."),
    (("find my cv", "~/Dokumenty/CV Jakub 2026.pdf"), "open it",
     T("open_file", "Opened CV Jakub 2026.pdf in Papers.", path="~/Dokumenty/CV Jakub 2026.pdf"), "Opened your CV."),
    (("znajdź fakturę z listopada", "~/Pobrane/faktura-11.pdf"), "otwórz ją",
     T("open_file", "Opened faktura-11.pdf in Papers.", path="~/Pobrane/faktura-11.pdf"), "Otworzyłem fakturę."),
    (("find my cv", "~/Dokumenty/CV Jakub 2026.pdf"), "show it in files",
     T("show_in_files", "Showed CV Jakub 2026.pdf in Files.", path="~/Dokumenty/CV Jakub 2026.pdf"), "Shown in Files."),
    (("timer for 10 minutes", "Timer set for 10 minutes."), "cancel it",
     T("cancel_reminder", "Cancelled: timer (14:15).", which="timer"), "Cancelled the timer."),
    (("minutnik na 15 minut", "Minutnik na 15 minut nastawiony."), "anuluj go",
     T("cancel_reminder", "Cancelled: timer (14:20).", which="timer"), "Anulowałem minutnik."),
    (("dark mode on", "Dark mode on."), "actually, turn it off",
     T("set_dark_mode", "Dark mode off.", enabled=False), None),
    (("ustaw jasność na 50", "Jasność ustawiona na 50%."), "jeszcze jaśniej",
     T("set_brightness", "Brightness set to 70%.", change="up"), "Rozjaśniłem do 70%."),
    (("play some Queen", "Playing 18 songs (Queen) in Gapless."), "skip this one",
     T("media_control", "Next track in Gapless: Don't Stop Me Now.", action="next_track"), None),
    (("puść Daft Punk", "Puszczam Daft Punk w Gapless."), "co to za piosenka?",
     T("media_control", "Gapless is playing “Instant Crush” by Daft Punk.", action="status"), "„Instant Crush”, Daft Punk."),
    (("what's on today", "Stand-up at 10:00 and the dentist at 14:30."), "move the dentist to 16:00",
     T("create_event", "Added “Dentist” on Monday 2026-11-16 16:00–17:00 to Personal.", title="Dentist", start="2026-11-16 16:00"),
     "Added the dentist at 16:00; delete the 14:30 one in Calendar if it's still there."),
    (("send the invoice to my phone", "Sent faktura-11.pdf to your iPhone."), "and to Ola by email",
     T("find_contact", "Ola Kowalska <ola.kowalska@example.com>", name="Ola"), None),
    (("ile to 23% z 1840?", "To 423,2."), "a 8%?",
     T("calculate", "8% * 1840 = 147.2", expression="8% * 1840"), "To 147,2."),
    (("what's 15% of 80", "12"), "and of 120?",
     T("calculate", "15% * 120 = 18", expression="15% * 120"), "18"),
]
for (u0, a0), u1, step, ans in FOLLOW:
    if step.tool == "find_contact":
        ex(u1, step,
           T("draft_email", "Opened a draft to ola.kowalska@example.com in Convey.", to="ola.kowalska@example.com", subject="Faktura", body="Hi Ola, here's the November invoice: faktura-11.pdf."),
           answer="The draft to Ola is open in Convey; attach faktura-11.pdf from Downloads and send.", before=[(u0, a0)])
    else:
        ex(u1, step, answer=ans, before=[(u0, a0)])

# ------------------------------------------------------------ everyday Polish, more forms
for u, tool, args, res, ans in [
    ("wyłącz ekran na noc, ściemnij maksymalnie", "set_brightness", {"percent": 1}, "Brightness set to 1%.", "Ściemniłem ekran do minimum."),
    ("pokaż mi foldery w dokumentach", "list_folder", {"path": "Documents"}, "~/Dokumenty (4 items): praca/  rachunki/  CV Jakub 2026.pdf  polisa OC 2026.pdf", "Foldery praca i rachunki, oraz CV i polisa."),
    ("zrób folder Rachunki 2026 w dokumentach", "make_folder", {"path": "Documents/Rachunki 2026"}, "Made ~/Dokumenty/Rachunki 2026.", "Zrobiłem folder Rachunki 2026 w Dokumentach."),
    ("skopiuj mi do schowka mój numer konta 12 3456 7890", "copy_to_clipboard", {"text": "12 3456 7890"}, "Copied to the clipboard.", "Skopiowane."),
    ("otwórz stronę allegro", "open_url", {"url": "https://allegro.pl"}, "Opened https://allegro.pl in Firefox.", "Otworzyłem Allegro."),
    ("włącz piosenkę Lewitacja", "play_music", {"query": "Lewitacja"}, "Playing Lewitacja by Dawid Podsiadło (~/Muzyka/Dawid Podsiadło/Lewitacja.flac) in Gapless.", "Gra „Lewitacja”."),
    ("pokaż sieci wifi", "list_networks", {}, "Networks: Home 5G (saved, connected, 90%), Biuro-Gosc (saved, 38%)", "Połączony z Home 5G; w zasięgu jest też Biuro-Gosc."),
    ("połącz z siecią w biurze", "connect_network", {"name": "Biuro-Gosc"}, "Connected to Biuro-Gosc.", "Połączyłem z Biuro-Gosc."),
    ("jak jest w Krakowie?", "get_weather", {"place": "Kraków"}, "Kraków now: 5°C, sunny.\nTomorrow: 3–9°C, cloudy.", "W Krakowie 5°C i słonecznie."),
    ("przypomnij mi za godzinę, żeby wyjąć pranie", "set_reminder", {"in_minutes": 60, "text": "wyjąć pranie"}, "I'll remind you in 60 minutes: wyjąć pranie.", "Przypomnę ci za godzinę."),
]:
    ex(u, T(tool, res, **args), answer=ans)

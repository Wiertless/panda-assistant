"""Hand-written colloquial requests (Claude, 2026-11-24): how people actually talk to
an assistant, mostly Polish ("weź mi…", "dasz radę…", "zerknij…"), where the small
model's Polish slips show most; a few English ones in the same spirit.
"""

from data_dsl import T, ex

PHONE = "jakubs-iphone"

ex("weź mi ściemnij ten ekran", T("set_brightness", "Brightness set to 30%.", change="down"), answer="Przyciemniłem do 30%.")
ex("dasz radę podgłośnić?", T("set_volume", "Volume set to 65%.", change="up"), answer="Jasne, 65%.")
ex("zerknij, czy mam coś jutro rano", T("get_events", "Tomorrow, Tuesday 2026-11-17:\n08:30–09:00 Spotkanie z promotorem\n14:00–15:00 Laboratorium", when="tomorrow"),
   answer="Rano o 8:30 spotkanie z promotorem, potem laborka o 14:00.")
ex("ogarnij mi minutnik na kwadrans", T("set_reminder", "Timer set for 15 minutes.", in_minutes=15), answer="Minutnik na 15 minut.")
ex("puść coś do pracy, jakiś spokojny album", T("play_music", "Playing 10 songs (Random Access Memories) in Gapless.", query="Random Access Memories"),
   answer="Puszczam „Random Access Memories” Daft Punk.")
ex("sprawdź, ile mam jeszcze baterii, bo zaraz wychodzę", T("get_battery", "Battery 57%, discharging, about 3 h 15 min left."),
   answer="57%, starczy na jakieś 3 godziny.")
ex("wrzuć mi na telefon tę fakturę z pobranych",
   T("find_files", "~/Pobrane/faktura-11.pdf (88 KB, 2026-11-14)", name="faktura", folder="Downloads"),
   T("send_file", f"Sent faktura-11.pdf to {PHONE} with Taildrop.", device=PHONE, path="~/Pobrane/faktura-11.pdf"),
   answer="Wrzuciłem fakturę na iPhone'a.")
ex("odpal mi terminal w projekcie sklep", T("open_terminal", "Opened a terminal in ~/Projekty/sklep.", folder="~/Projekty/sklep"),
   answer="Terminal otwarty w Projekty/sklep.")
ex("zgaś to wifi na chwilę", T("set_wifi", "Wi-Fi off.", enabled=False), answer="Wi-Fi wyłączone.")
ex("a teraz z powrotem włącz", T("set_wifi", "Wi-Fi on.", enabled=True), answer="Wi-Fi włączone.",
   before=[("zgaś to wifi na chwilę", "Wi-Fi wyłączone.")])
ex("co tam w mailach od uczelni?",
   T("search_mail", "Mail matching “uczelnia”, newest first:\n[uni:101] Dziekanat — Odwołane zajęcia (Monday 2026-11-16 09:30, the newest)\n  Środowe ćwiczenia z fizyki są odwołane.", query="uczelnia"),
   answer="Dziekanat pisze, że środowe ćwiczenia z fizyki są odwołane.")
ex("zapisz se, że jutro idę do fryzjera o 15",
   T("create_event", "Added “Fryzjer” on Monday 2026-11-16 15:00–16:00 to Personal.", title="Fryzjer", start="2026-11-16 15:00"),
   answer="Wpisałem fryzjera jutro na 15:00.")
ex("zmniejsz mi te fotki z kota, za duże są", T("edit_image", "Saved 8 of 8 in ~/Obrazy/Edited (the originals are unchanged):\n~/Obrazy/Edited/kot1.jpg: 4.0 MB 4032×3024 → 0.6 MB 1920×1440\n…",
   files="Obrazy/Kot", resize="1920"), answer="Gotowe, 8 mniejszych zdjęć kota w Obrazy/Edited.")
ex("ile to wychodzi na łebka, jak rachunek to 342 zł, a nas jest czworo?",
   T("calculate", "342 / 4 = 85.5", expression="342 / 4"), answer="Po 85,50 zł na osobę.")
ex("jak tam pogoda u mnie?", T("get_weather", "Toruń now: 3°C, drizzle.\nTomorrow: 1–6°C, cloudy.", place="Toruń"),
   answer="W Toruniu 3°C i mżawka.")
ex("weź otwórz mi ten artykuł o Fedorze z wczoraj",
   T("web_search", "Results from DuckDuckGo for 'Fedora artykuł': 1. Fedora 45 is here - fedoramagazine.org/announcing-fedora-45 …", query="Fedora artykuł"),
   T("open_url", "Opened https://fedoramagazine.org/announcing-fedora-45 in Firefox.", url="https://fedoramagazine.org/announcing-fedora-45"),
   answer="Otworzyłem „Fedora 45 is here” z Fedora Magazine.")
ex("przypomnij mi pojutrze rano o wizycie u lekarza",
   T("set_reminder", "I'll remind you on Tuesday at 08:00: wizyta u lekarza.", at="2026-11-17 08:00", text="wizyta u lekarza"),
   answer="Przypomnę we wtorek o 8:00.")
ex("wycisz wszystko, mam spotkanie", T("set_mute", "Sound muted.", muted=True), T("set_do_not_disturb", "Do Not Disturb on.", enabled=True),
   answer="Wyciszyłem dźwięk i powiadomienia.")
ex("spotkanie się skończyło, przywróć dźwięk i powiadomienia",
   T("set_mute", "Sound unmuted.", muted=False), T("set_do_not_disturb", "Do Not Disturb off.", enabled=False),
   answer="Dźwięk i powiadomienia znowu włączone.")
ex("hej, wyślij Oli maila, że się spóźnię 10 minut",
   T("find_contact", "Ola Kowalska <ola.kowalska@example.com>", name="Ola"),
   T("draft_email", "Opened a draft to ola.kowalska@example.com in Convey.", to="ola.kowalska@example.com", subject="Spóźnię się", body="Cześć Ola, spóźnię się około 10 minut. Przepraszam!"),
   answer="Szkic do Oli jest gotowy w Convey; sprawdź i wyślij.")
ex("czy ten pociąg o 15:12 do Gdańska to dzisiaj?",
   T("search_mail", "Mail matching “PKP”, newest first:\n[home:520] PKP Intercity — Bilet: Toruń → Gdańsk (Saturday 2026-11-14 12:00, the newest)\n  Pociąg 15:12 w poniedziałek 16.11, wagon 7, miejsce 54.", query="PKP"),
   answer="Nie, w poniedziałek 16 listopada (wagon 7, miejsce 54).")

# English, same spirit
ex("can you just make it quieter", T("set_volume", "Volume set to 35%.", change="down"))
ex("grab me the weather for Lisbon, I fly there friday",
   T("get_weather", "Lisbon now: 17°C, sunny.\nTomorrow: 15–19°C, sunny.\nWednesday: 14–18°C, showers.", place="Lisbon"),
   answer="17°C and sunny in Lisbon now; the forecast shown goes to Wednesday (showers), so check again closer to Friday.")
ex("shoot the boarding pass over to my phone",
   T("find_files", "~/Pobrane/boarding pass LIS.pdf (180 KB, 2026-11-13)", name="boarding pass"),
   T("send_file", f"Sent boarding pass LIS.pdf to {PHONE} with Taildrop.", device=PHONE, path="~/Pobrane/boarding pass LIS.pdf"),
   answer="Sent the boarding pass to your iPhone.")
ex("mute everything, I'm in a meeting", T("set_mute", "Sound muted.", muted=True), T("set_do_not_disturb", "Do Not Disturb on.", enabled=True),
   answer="Muted the sound and notifications.")
ex("lemme know in 20 if I'm still on reddit", T("set_reminder", "I'll remind you in 20 minutes: get off Reddit.", in_minutes=20, text="get off Reddit"))

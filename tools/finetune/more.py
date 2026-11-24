"""The last hand-written batch for the first big run (Claude, 2026-11-24): settings
combined in one request, recovery in Polish, and a few longer real tasks.
"""

from data_dsl import T, ex

PHONE = "jakubs-iphone"

# ------------------------------------------------------------ two settings in one request
ex("dark mode and night light on please",
   T("set_dark_mode", "Dark mode on.", enabled=True), T("set_night_light", "Night Light on.", enabled=True),
   answer="Dark mode and Night Light are on.")
ex("włącz tryb ciemny i ścisz do 30",
   T("set_dark_mode", "Dark mode on.", enabled=True), T("set_volume", "Volume set to 30%.", percent=30),
   answer="Ciemny motyw włączony, głośność 30%.")
ex("I'm going to sleep: dim the screen, mute and do not disturb",
   T("set_brightness", "Brightness set to 10%.", percent=10), T("set_mute", "Sound muted.", muted=True),
   T("set_do_not_disturb", "Do Not Disturb on.", enabled=True),
   answer="Screen dimmed to 10%, sound muted, Do Not Disturb on. Good night!")
ex("idę spać: przyciemnij ekran i włącz nie przeszkadzać",
   T("set_brightness", "Brightness set to 10%.", percent=10), T("set_do_not_disturb", "Do Not Disturb on.", enabled=True),
   answer="Ekran przyciemniony, tryb Nie przeszkadzać włączony. Dobranoc!")
ex("work mode: performance, wifi on, open firefox",
   T("set_power_mode", "Power mode: performance.", mode="performance"), T("set_wifi", "Wi-Fi on.", enabled=True),
   T("open_app", "Opened Firefox.", name="Firefox"),
   answer="Performance mode, Wi-Fi on, Firefox open.")
ex("wyłącz bluetooth i wifi, lecę samolotem",
   T("set_bluetooth", "Bluetooth off.", enabled=False), T("set_wifi", "Wi-Fi off.", enabled=False),
   answer="Bluetooth i Wi-Fi wyłączone. Miłego lotu!")

# ------------------------------------------------------------ recovery, in Polish
ex("wyślij notatki na iphona",
   T("send_file", "Error: no device called iphona. Devices: jakubs-iphone (iOS, online), framework (linux, this computer).", device="iphona", path="Documents/notes.txt"),
   T("send_file", f"Sent notes.txt to {PHONE} with Taildrop.", device=PHONE, path="Documents/notes.txt"),
   answer="Wysłałem notatki na iPhone'a.")
ex("otwórz gimpa",
   T("open_app", "Error: no app called gimpa. Close names: GIMP.", name="gimpa"),
   T("open_app", "Opened GIMP.", name="GIMP"), answer="Otworzyłem GIMP-a.")
ex("puść album Discovery",
   T("play_music", "Error (retrying won't help): no song, album or artist matching “Discovery” in your music library", query="Discovery"),
   answer="Nie mam „Discovery” w twojej bibliotece muzyki; odtwarzam tylko to, co jest w folderze Muzyka.")
ex("połącz słuchawki",
   T("connect_bluetooth", "Error (retrying won't help): Sony WH-1000XM5 is off or out of range.", device="Sony WH-1000XM5"),
   answer="Słuchawki Sony są wyłączone albo poza zasięgiem; włącz je i spróbuj jeszcze raz.",
   before=[("które słuchawki mam?", "Sony WH-1000XM5 i AirPods Pro.")])
ex("usuń plik stary.txt z pulpitu",
   T("trash_file", "Error: Desktop/stary.txt doesn't exist. On the Desktop: scan.jpg, todo.txt, Zrzut ekranu.png.", path="Desktop/stary.txt"),
   answer="Na pulpicie nie ma stary.txt; są tylko scan.jpg, todo.txt i zrzut ekranu.")

# ------------------------------------------------------------ longer real tasks
ex("plan my evening: remind me at 19 to cook, at 21 to call mum, and turn on night light at 22",
   T("set_reminder", "I'll remind you today at 19:00: cook dinner.", at="2026-11-15 19:00", text="cook dinner"),
   T("set_reminder", "I'll remind you today at 21:00: call mum.", at="2026-11-15 21:00", text="call mum"),
   T("set_reminder", "I'll remind you today at 22:00: turn on Night Light.", at="2026-11-15 22:00", text="turn on Night Light"),
   answer="Reminders at 19:00 (cook) and 21:00 (call mum), and one at 22:00 for Night Light: I can't switch it on by myself later, so I'll remind you.")
ex("przygotuj mi wyjazd: pogoda w Gdańsku na jutro i wyślij bilet na telefon",
   T("get_weather", "Gdańsk now: 5°C, windy.\nTomorrow: 3–7°C, rain.", place="Gdańsk"),
   T("search_mail", "Mail matching “bilet”, newest first:\n[home:520] PKP Intercity — Bilet: Toruń → Gdańsk (Saturday 2026-11-14 12:00, the newest)\n  Bilet w załączniku: bilet-PKP-88231.pdf.", query="bilet"),
   T("find_files", "~/Pobrane/bilet-PKP-88231.pdf (95 KB, 2026-11-14)", name="bilet-PKP"),
   T("send_file", f"Sent bilet-PKP-88231.pdf to {PHONE} with Taildrop.", device=PHONE, path="~/Pobrane/bilet-PKP-88231.pdf"),
   answer="Jutro w Gdańsku deszcz, 3–7°C, weź parasol. Bilet wysłałem na iPhone'a.")
ex("back up my thesis: copy it to the thesis folder in documents with today's date",
   T("find_files", "~/Dokumenty/praca/praca magisterska v7.docx (1.4 MB, 2026-11-12)", name="praca magisterska"),
   T("make_folder", "Made ~/Dokumenty/praca/kopie.", path="Documents/praca/kopie"),
   T("run_command", "", command="cp '/var/home/kuba/Dokumenty/praca/praca magisterska v7.docx' '/var/home/kuba/Dokumenty/praca/kopie/praca magisterska 2026-11-16.docx'"),
   answer="Copied it to Dokumenty/praca/kopie/praca magisterska 2026-11-16.docx.")
ex("what's the total of my last two invoices",
   T("search_mail", "Mail matching “faktura”, newest first:\n[home:661] Play — Twoja faktura (Wednesday 2026-11-11 09:00, the newest)\n  Kwota: 45,00 zł.\n[home:640] Orange — Faktura za październik (Friday 2026-11-06 08:00)\n  Do zapłaty: 89,99 zł.", query="faktura"),
   T("calculate", "89.99 + 45 = 134.99", expression="89.99 + 45"),
   answer="134.99 zł: Play 45 zł and Orange 89.99 zł.")

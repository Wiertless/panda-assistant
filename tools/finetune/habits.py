"""Hand-written conversations for the habit the eval shows Gemma lacks most (30 of
59 failures on the NPU, 2026-11-24): answering from thin air instead of looking.
The user's own files, documents, contacts, mail, live facts and settings always
go through a tool first, and an action is only "done" when a tool did it.
Same kinds of situations as the eval's failures, other documents, people and
wordings (data.py rejects any eval prompt). Written by Claude.
"""

from data_dsl import T, chat, ex

FOUND_POLICY = "~/Dokumenty/polisa OC 2026.pdf (640 KB, 2026-01-15)"

# ------------------------------------------------------------ questions about the user's documents
ex("how much is the excess on my car insurance",
   T("find_files", FOUND_POLICY, name="polisa"),
   T("ask_document", "From ~/Dokumenty/polisa OC 2026.pdf: § 9. Udział własny w szkodzie wynosi 500 zł.",
     path="~/Dokumenty/polisa OC 2026.pdf", question="how much is the excess?"),
   answer="500 zł, per the policy (§ 9).")
ex("do kiedy mam ważną polisę na samochód?",
   T("find_files", FOUND_POLICY, name="polisa"),
   T("ask_document", "From ~/Dokumenty/polisa OC 2026.pdf: Okres ubezpieczenia: od 15.01.2026 do 14.01.2027.",
     path="~/Dokumenty/polisa OC 2026.pdf", question="do kiedy jest ważna polisa?"),
   answer="Do 14 stycznia 2027.")
ex("what's the warranty on my laptop according to the invoice",
   T("find_files", "~/Dokumenty/faktura Framework.pdf (210 KB, 2025-12-03)", name="faktura Framework"),
   T("ask_document", "From ~/Dokumenty/faktura Framework.pdf: Gwarancja: 24 miesiące od daty zakupu (03.12.2025).",
     path="~/Dokumenty/faktura Framework.pdf", question="how long is the warranty?"),
   answer="24 months from 3 December 2025, so until December 2027.")
ex("ile wynosi okres wypowiedzenia w mojej umowie o pracę?",
   T("find_files", "~/Dokumenty/praca/umowa o pracę 2025.pdf (330 KB, 2025-03-01)", name="umowa o pracę"),
   T("ask_document", "From ~/Dokumenty/praca/umowa o pracę 2025.pdf: § 7. Okres wypowiedzenia wynosi 1 miesiąc.",
     path="~/Dokumenty/praca/umowa o pracę 2025.pdf", question="ile wynosi okres wypowiedzenia?"),
   answer="Miesiąc (§ 7 umowy).")
ex("how many GB does my phone plan include",
   T("find_files", "~/Pobrane/umowa Play abonament.pdf (520 KB, 2026-04-11)", name="umowa Play"),
   T("ask_document", "From ~/Pobrane/umowa Play abonament.pdf: Pakiet danych: 60 GB miesięcznie, roaming w UE 21 GB.",
     path="~/Pobrane/umowa Play abonament.pdf", question="how many GB are included?"),
   answer="60 GB a month, 21 GB of it usable in EU roaming.")
ex("summarize the meeting notes from monday",
   T("find_files", "~/Dokumenty/notatki spotkanie 2026-11-16.md (6 KB, 2026-11-16)", name="notatki spotkanie"),
   T("ask_document", "From ~/Dokumenty/notatki spotkanie 2026-11-16.md: Decided to move the release to December; Ola owns the migration; next sync on Thursday.",
     path="~/Dokumenty/notatki spotkanie 2026-11-16.md", question="summarize"),
   answer="The release moves to December, Ola owns the migration, and the next sync is on Thursday.")
# a document that isn't there: say so, don't make it up
ex("what does my rental insurance cover",
   T("find_files", "No files named like “rental insurance”. Similar: polisa OC 2026.pdf (~/Dokumenty).", name="rental insurance"),
   answer="I couldn't find a rental insurance document; the only policy I see is the car one, polisa OC 2026.pdf. Is it saved somewhere else?")

# ------------------------------------------------------------ open it or show it, not describe it
ex("open my car insurance",
   T("find_files", FOUND_POLICY, name="polisa"),
   T("open_file", "Opened polisa OC 2026.pdf in Papers.", path="~/Dokumenty/polisa OC 2026.pdf"),
   answer="Opened polisa OC 2026.pdf.")
ex("otwórz fakturę za laptopa",
   T("find_files", "~/Dokumenty/faktura Framework.pdf (210 KB, 2025-12-03)", name="faktura Framework"),
   T("open_file", "Opened faktura Framework.pdf in Papers.", path="~/Dokumenty/faktura Framework.pdf"),
   answer="Otworzyłem fakturę.")
ex("show me where my thesis is saved",
   T("find_files", "~/Dokumenty/praca/praca magisterska v7.docx (1.4 MB, 2026-11-12)", name="praca magisterska"),
   T("show_in_files", "Showed praca magisterska v7.docx in Files.", path="~/Dokumenty/praca/praca magisterska v7.docx"),
   answer="It's in Dokumenty/praca; I've shown it in Files.")
ex("pokaż w plikach, gdzie jest moje CV",
   T("find_files", "~/Dokumenty/CV Jakub 2026.pdf (310 KB, 2026-10-02)", name="CV"),
   T("show_in_files", "Showed CV Jakub 2026.pdf in Files.", path="~/Dokumenty/CV Jakub 2026.pdf"),
   answer="Pokazałem CV w Plikach (folder Dokumenty).")
ex("where did I save the boarding pass",
   T("find_files", "~/Pobrane/boarding pass LIS.pdf (180 KB, 2026-11-13)", name="boarding pass"),
   answer="In Downloads: boarding pass LIS.pdf.")
ex("co mam na pulpicie?",
   T("list_folder", "~/Pulpit (3 items): scan.jpg (1.2 MB, 2026-11-15)  todo.txt (1 KB, 2026-11-14)  Zrzut ekranu.png (640 KB, 2026-11-13)", path="Desktop"),
   answer="Trzy pliki: scan.jpg, todo.txt i zrzut ekranu.")
ex("move scan.jpg from the desktop to my documents",
   T("move_file", "Moved ~/Pulpit/scan.jpg to ~/Dokumenty/scan.jpg.", **{"from": "Desktop/scan.jpg", "to": "Documents"}),
   answer="Moved scan.jpg to Documents.")
ex("przenieś todo.txt z pulpitu do dokumentów",
   T("move_file", "Moved ~/Pulpit/todo.txt to ~/Dokumenty/todo.txt.", **{"from": "Desktop/todo.txt", "to": "Documents"}),
   answer="Przeniosłem todo.txt do Dokumentów.")

# ------------------------------------------------------------ contacts
ex("what's Marek's phone number",
   T("find_contact", "Marek Nowak: +48 601 234 567, marek.nowak@example.com", name="Marek"),
   answer="Marek Nowak: +48 601 234 567.")
ex("jaki jest mail do Oli?",
   T("find_contact", "Ola Kowalska <ola.kowalska@example.com>, +48 512 345 678", name="Ola"),
   answer="ola.kowalska@example.com")
ex("text my landlord's number to my phone",
   T("find_contact", "Marek Nowak (landlord): +48 601 234 567", name="landlord"),
   T("send_text", "Sent the note to jakubs-iphone with Taildrop.", device="jakubs-iphone", text="Marek Nowak (landlord): +48 601 234 567", title="Landlord"),
   answer="Sent Marek's number to your iPhone.")
ex("write to my brother that dinner is at 7",
   T("find_contact", "Piotr (brother) <piotr.w@example.com>", name="brother"),
   T("draft_email", "Opened a draft to piotr.w@example.com in Convey.", to="piotr.w@example.com", subject="Dinner", body="Hi Piotr, dinner is at 7."),
   answer="The draft to Piotr is open in Convey; check it and send.")

# ------------------------------------------------------------ live facts: look them up
ex("how much is 50 dollars in złoty today",
   T("web_search", "Results from DuckDuckGo for 'USD PLN exchange rate today': 1. 1 USD = 3.98 PLN - NBP table 225/A/NBP/2026 (2026-11-16)…", query="USD PLN exchange rate today"),
   T("calculate", "50 * 3.98 = 199", expression="50 * 3.98"),
   answer="About 199 zł (NBP rate 3.98 today).")
ex("ile kosztuje dzisiaj euro?",
   T("web_search", "Results from DuckDuckGo for 'kurs euro dzisiaj': 1. EUR 4,26 zł - kurs średni NBP (2026-11-16)…", query="kurs euro dzisiaj"),
   answer="Średni kurs NBP: 4,26 zł za euro.")
ex("is there a new GNOME release",
   T("web_search", "Results from DuckDuckGo for 'GNOME latest release': 1. GNOME 50 released - gnome.org - September 2026…", query="GNOME latest release"),
   answer="GNOME 50, released in September 2026 (gnome.org).")
ex("what's the newest version of the linux kernel",
   T("web_search", "Results from DuckDuckGo for 'latest linux kernel version': 1. The Linux Kernel Archives - kernel.org - stable: 7.3.6…", query="latest linux kernel version"),
   answer="Stable is 7.3.6 (kernel.org).")
ex("kiedy jest najbliższy pociąg z Torunia do Gdańska?",
   T("web_search", "Results from DuckDuckGo for 'pociąg Toruń Gdańsk rozkład': 1. Rozkład jazdy PKP Intercity - Toruń Gł. → Gdańsk Gł.: 10:42, 12:15, 14:05…", query="pociąg Toruń Gdańsk rozkład"),
   answer="Według rozkładu PKP Intercity najbliższe są o 10:42, potem 12:15.")

# ------------------------------------------------------------ settings found by name, then changed
SETTINGS = [
    ("show seconds on the clock", "clock seconds",
     "org.gnome.desktop.interface clock-show-seconds (boolean, now false): seconds in the top bar clock",
     "org.gnome.desktop.interface", "clock-show-seconds", "true", "The clock now shows seconds."),
    ("pokaż dzień tygodnia przy zegarze", "clock weekday",
     "org.gnome.desktop.interface clock-show-weekday (boolean, now false): weekday in the top bar clock",
     "org.gnome.desktop.interface", "clock-show-weekday", "true", "Zegar pokazuje teraz dzień tygodnia."),
    ("turn off the hot corner", "hot corner",
     "org.gnome.desktop.interface enable-hot-corners (boolean, now true): the top-left hot corner",
     "org.gnome.desktop.interface", "enable-hot-corners", "false", "The hot corner is off."),
    ("make everything on screen a bit larger", "text scaling",
     "org.gnome.desktop.interface text-scaling-factor (double, now 1.0): text size",
     "org.gnome.desktop.interface", "text-scaling-factor", "1.15", "Text is now 15% larger."),
    ("wyłącz dźwięki alertów", "event sounds",
     "org.gnome.desktop.sound event-sounds (boolean, now true): alert and event sounds",
     "org.gnome.desktop.sound", "event-sounds", "false", "Dźwięki alertów wyłączone."),
    ("tap to click on the touchpad", "touchpad tap",
     "org.gnome.desktop.peripherals.touchpad tap-to-click (boolean, now false): tap to click",
     "org.gnome.desktop.peripherals.touchpad", "tap-to-click", "true", "Tap to click is on."),
]
for user, query, found, schema, key, value, answer in SETTINGS:
    ex(user,
       T("find_settings", found, query=query),
       T("change_setting", f"{key} set to {value}.", schema=schema, key=key, value=value),
       answer=answer)

# ------------------------------------------------------------ devices by what they are
ex("connect my mouse",
   T("connect_bluetooth", "Connected to MX Master 3.", device="MX Master 3"))
ex("podłącz moje słuchawki sony",
   T("connect_bluetooth", "Connected to Sony WH-1000XM5.", device="Sony WH-1000XM5"),
   answer="Połączyłem słuchawki Sony.")
ex("disconnect the earbuds",
   T("disconnect_bluetooth", "Disconnected Jakub's AirPods Pro.", device="Jakub's AirPods Pro"))
ex("get me online through the phone hotspot",
   T("connect_network", "Connected to Phone hotspot.", name="Phone hotspot"))
ex("tryb oszczędzania energii",
   T("set_power_mode", "Power mode: power-saver.", mode="power-saver"),
   answer="Włączyłem oszczędzanie energii.")
ex("I need full speed for compiling",
   T("set_power_mode", "Power mode: performance.", mode="performance"))
ex("wyłącz tryb nie przeszkadzać",
   T("set_do_not_disturb", "Do Not Disturb off.", enabled=False),
   answer="Powiadomienia są znowu włączone.")

# ------------------------------------------------------------ calendar with details
ex("hairdresser on friday at 4pm at Szeroka 12",
   T("create_event", "Added “Hairdresser” on Friday 2026-11-20 16:00–17:00 at Szeroka 12 to Personal.",
     title="Hairdresser", start="2026-11-20 16:00", place="Szeroka 12"),
   answer="Added the hairdresser on Friday at 16:00, Szeroka 12.")
ex("wpisz mi w czwartek od 18 do 20 naukę do egzaminu",
   T("create_event", "Added “Nauka do egzaminu” on Thursday 2026-11-19 18:00–20:00 to Personal.",
     title="Nauka do egzaminu", start="2026-11-19 18:00", end="2026-11-19 20:00"),
   answer="Dodałem naukę w czwartek 18:00–20:00.")
ex("remind me at 7:30 tomorrow to bring the documents",
   T("set_reminder", "I'll remind you on Monday at 07:30: bring the documents.", at="2026-11-16 07:30", text="bring the documents"))
ex("przypomnij mi w piątek o 17 o prezencie dla Ani",
   T("set_reminder", "I'll remind you on Friday at 17:00: prezent dla Ani.", at="2026-11-20 17:00", text="prezent dla Ani"),
   answer="Przypomnę ci w piątek o 17:00.")

# ------------------------------------------------------------ chains that need the first result
ex("check if it rains in Gdańsk tomorrow and if so remind me at 7 to grab an umbrella",
   T("get_weather", "Gdańsk now: 7°C, cloudy.\nTomorrow: 5–9°C, rain from the morning.", place="Gdańsk"),
   T("set_reminder", "I'll remind you on Monday at 07:00: grab an umbrella.", at="2026-11-16 07:00", text="grab an umbrella"),
   answer="Rain is forecast, so I'll remind you at 7:00 to grab an umbrella.")
ex("jeśli jutro w Krakowie będzie słońce, przypomnij mi o 9 o rowerze",
   T("get_weather", "Kraków now: 4°C, clear.\nTomorrow: 2–10°C, rain all day.", place="Kraków"),
   answer="Jutro w Krakowie ma padać cały dzień, więc nie ustawiłem przypomnienia o rowerze.")
ex("put the date from Ola's last email about the party in my calendar",
   T("search_mail", "Mail matching “Ola party”, newest first:\n[home:220] Ola Kowalska — Impreza urodzinowa (Sunday 2026-11-15 12:10, the newest)\n  Zapraszam w sobotę 28 listopada o 19:00, ul. Mostowa 3.", query="Ola party"),
   T("create_event", "Added “Ola's birthday party” on Saturday 2026-11-28 19:00–22:00 at ul. Mostowa 3 to Personal.",
     title="Ola's birthday party", start="2026-11-28 19:00", place="ul. Mostowa 3"),
   answer="Added Ola's party on Saturday 28 November at 19:00, ul. Mostowa 3.")
ex("what did the dentist's office email about, and add it to the calendar",
   T("search_mail", "Mail matching “dentist”, newest first:\n[home:216] Gabinet Uśmiech — Przypomnienie o wizycie (Friday 2026-11-13 09:00, the newest)\n  Wizyta kontrolna we wtorek 24.11 o 10:30.", query="dentist"),
   T("create_event", "Added “Dentist check-up” on Tuesday 2026-11-24 10:30–11:30 to Personal.",
     title="Dentist check-up", start="2026-11-24 10:30"),
   answer="A check-up reminder: Tuesday 24 November at 10:30. It's in your calendar now.")
ex("read the latest email from the university",
   T("search_mail", "Mail matching “university”, newest first:\n[uni:91] Dziekanat — Zmiana sali (Monday 2026-11-16 07:50, the newest)\n  Zajęcia z analizy...", query="university"),
   T("read_mail", "From: Dziekanat <dziekanat@uni.example.pl>\nSubject: Zmiana sali\nDate: Monday 2026-11-16 07:50\n\nZajęcia z analizy matematycznej w środę odbędą się w sali 204 zamiast 112.", id="uni:91"),
   answer="The dean's office: Wednesday's calculus class moves to room 204 (instead of 112).")
ex("copy the code from the last email from the bank",
   T("search_mail", "Mail matching “bank”, newest first:\n[home:225] mBank — Kod weryfikacyjny (Monday 2026-11-16 10:02, the newest)\n  Twój kod: 482913. Ważny 10 minut.", query="bank"),
   T("copy_to_clipboard", "Copied to the clipboard.", text="482913"),
   answer="Copied the code 482913.")

# ------------------------------------------------------------ apps by what they do
ex("is there an app here for drawing",
   T("search_apps", "Apps matching “drawing”: Drawing (com.github.maoschanz.drawing), Krita, Pinta.", query="drawing"),
   answer="Yes: Drawing, Krita and Pinta are installed.")
ex("czym mogę tu edytować wideo?",
   T("search_apps", "Apps matching “video editor”: Pitivi, Kdenlive.", query="video editor"),
   answer="Masz Pitivi i Kdenlive.")

# ------------------------------------------------------------ things to remember, said in passing
ex("by the way, I'm vegetarian", T("remember", "Remembered: The user is vegetarian.", fact="The user is vegetarian"),
   answer="Noted, I'll keep that in mind.")
ex("mam na imię Kuba", T("remember", "Remembered: The user's name is Kuba.", fact="The user's name is Kuba"),
   answer="Miło mi, Kuba!")
ex("my sister is called Ola", T("remember", "Remembered: The user's sister is called Ola.", fact="The user's sister is called Ola"),
   answer="Got it.")

# ------------------------------------------------------------ undo, by words
ex("change that back", T("undo", "Undid “Volume 80%”: volume is back to 40%."),
   before=[("volume to 80", "Volume set to 80%.")])
ex("przywróć jak było", T("undo", "Undid “Text size 1.15”: text size is back to 1.0."),
   answer="Przywróciłem poprzedni rozmiar tekstu.",
   before=[("powiększ trochę tekst", "Tekst jest teraz o 15% większy.")])

# ------------------------------------------------------------ chat stays chat (no tools)
for q, a in [
    ("what's a good way to learn Polish", "Short daily practice: an app like Anki for words, a podcast for listening, and a weekly chat with a native speaker."),
    ("tell me something interesting", "Octopuses have three hearts, and two of them stop when they swim."),
    ("czy mógłbyś mi coś doradzić na nudę?", "Spacer, krótki film albo nowa piosenka do nauczenia; a może książka z półki, której jeszcze nie zacząłeś?"),
    ("why do cats purr", "Mostly to show they're content, but also to calm themselves when stressed; the vibration may even help healing."),
    ("jak się robi naleśniki?", "Szklanka mąki, szklanka mleka, pół szklanki wody, 2 jajka, szczypta soli; wymieszaj i smaż cienko na rozgrzanej patelni."),
    ("what's the difference between RAM and storage", "RAM is fast working memory that empties when the power goes off; storage (the SSD) keeps your files."),
    ("ok", "Okay! Anything else?"),
    ("nevermind", "No problem."),
    ("nieważne", "Jasne."),
    ("haha", "😄"),
]:
    chat(q, a)

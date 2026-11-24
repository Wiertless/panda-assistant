"""Hand-written realistic tasks (Claude, 2026-11-24): what someone would actually
ask a desktop assistant over a day, several steps when the task needs them, and
answers that pick out what matters from long tool results (the eval's "answer"
capability: 89/119). English and Polish.
"""

from data_dsl import T, ex

PHONE = "jakubs-iphone"

# ------------------------------------------------------------ mornings
ex("what's my day looking like",
   T("get_events", "Today, Monday 2026-11-16:\n09:00–09:15 Stand-up\n11:00–12:00 Code review with Ola\n15:30–16:00 Dentist (Gabinet Uśmiech)"),
   T("get_weather", "Toruń now: 2°C, fog.\nTomorrow: 0–5°C, cloudy.", place="Toruń"),
   answer="Stand-up at 9:00, code review with Ola at 11:00 and the dentist at 15:30. Foggy and 2°C outside.")
ex("dzień dobry, co dziś mam?",
   T("get_events", "Today, Monday 2026-11-16:\n10:00–12:00 Wykład: Analiza matematyczna\n17:00–18:30 Siłownia"),
   answer="Dzień dobry! Wykład z analizy o 10:00 i siłownia o 17:00.")
ex("did anything important come in overnight",
   T("search_mail", "Mail matching “”, newest first:\n[home:701] mBank — Wpłynął przelew (Monday 2026-11-16 06:12, the newest)\n  Wpłynęło 4 850,00 zł od: FIRMA SP. Z O.O.\n[home:700] Allegro — Promocje tygodnia (Monday 2026-11-16 05:00)\n  Do -40%…\n[uni:99] USOS — Nowa ocena (Sunday 2026-11-15 23:40)\n  Ocena 5,0 z przedmiotu Programowanie.", query=""),
   answer="Your salary came in (4,850 zł), and you got a 5.0 in Programming. The rest is Allegro promotions.")

# ------------------------------------------------------------ errands
ex("how much did I pay for internet last month",
   T("search_mail", "Mail matching “faktura internet”, newest first:\n[home:640] Orange — Faktura za październik (Friday 2026-11-06 08:00, the newest)\n  Do zapłaty: 89,99 zł, termin 25.11.2026.", query="faktura internet"),
   answer="89.99 zł for October (Orange), due by 25 November.")
ex("czy zapłaciłem już za prąd?",
   T("search_mail", "Mail matching “prąd”, newest first:\n[home:655] Energa — Potwierdzenie płatności (Thursday 2026-11-12 10:30, the newest)\n  Otrzymaliśmy płatność 212,40 zł za fakturę 10/2026.", query="prąd"),
   answer="Tak: Energa potwierdziła płatność 212,40 zł 12 listopada.")
ex("when is my parcel coming",
   T("search_mail", "Mail matching “paczka”, newest first:\n[home:705] InPost — Paczka w drodze (Monday 2026-11-16 08:40, the newest)\n  Paczka 6900 1234 dotrze dziś do Paczkomatu TOR12M (ul. Szosa Chełmińska 50).", query="paczka"),
   answer="Today, at the TOR12M parcel locker (ul. Szosa Chełmińska 50).")
ex("remind me to pick up the parcel at 6pm",
   T("set_reminder", "I'll remind you today at 18:00: pick up the parcel.", at="2026-11-16 18:00", text="pick up the parcel"))
ex("ile zostało mi do zapłaty za telefon i do kiedy?",
   T("search_mail", "Mail matching “Play faktura”, newest first:\n[home:661] Play — Twoja faktura (Wednesday 2026-11-11 09:00, the newest)\n  Kwota: 45,00 zł. Termin płatności: 23.11.2026.", query="Play faktura"),
   answer="45 zł, do 23 listopada.")

# ------------------------------------------------------------ work
ex("what did Ola say in her last email",
   T("search_mail", "Mail matching “Ola”, newest first:\n[work:310] Ola Kowalska — Re: release (Monday 2026-11-16 10:12, the newest)\n  Hi, I moved the release to Thursday; the migration needs one more day…", query="Ola"),
   T("read_mail", "From: Ola Kowalska <ola.kowalska@example.com>\nSubject: Re: release\nDate: Monday 2026-11-16 10:12\n\nHi, I moved the release to Thursday; the migration needs one more day. Could you review PR #212 by Wednesday noon? Thanks!", id="work:310"),
   answer="The release moves to Thursday (the migration needs a day more), and she asks you to review PR #212 by Wednesday noon.")
ex("put the PR review on wednesday morning in my calendar",
   T("create_event", "Added “Review PR #212” on Wednesday 2026-11-18 09:00–10:00 to Personal.", title="Review PR #212", start="2026-11-18 09:00"),
   answer="Added “Review PR #212” on Wednesday at 9:00.",
   before=[("what did Ola say in her last email", "The release moves to Thursday, and she asks you to review PR #212 by Wednesday noon.")])
ex("czy mój serwer deweloperski działa?",
   T("run_command", "kuba  4321  node /home/kuba/Projekty/sklep/node_modules/.bin/vite --port 5173", command="pgrep -a -f vite"),
   answer="Tak, Vite działa na porcie 5173.")
ex("how big is my projects folder",
   T("run_command", "38G\t/var/home/kuba/Projekty", command="du -sh ~/Projekty"),
   answer="38 GB.")
ex("what branch am I on in the sklep project",
   T("run_command", "feature/checkout", command="git branch --show-current", folder="~/Projekty/sklep"),
   answer="feature/checkout.")

# ------------------------------------------------------------ files, the long way round
ex("find the photo of the whiteboard from friday and make it a pdf",
   T("find_files", "~/Obrazy/IMG_2990.jpg (3.3 MB, 2026-11-13)\n~/Obrazy/IMG_2991.jpg (3.1 MB, 2026-11-13)", name="IMG_299", folder="Obrazy"),
   T("images_to_pdf", "Made ~/Obrazy/Edited/tablica.pdf from 2 pictures (1.4 MB)", files="Obrazy/IMG_299?.jpg", name="tablica"),
   answer="Friday had two photos (IMG_2990 and IMG_2991); both are in Obrazy/Edited/tablica.pdf.")
ex("send the last 3 photos from my phone folder to Ola",
   T("list_folder", "~/Pobrane/zdjęcia z telefonu (newest first): IMG_5512.jpg  IMG_5511.jpg  IMG_5510.jpg  IMG_5500.jpg …", path="Pobrane/zdjęcia z telefonu"),
   T("find_contact", "Ola Kowalska <ola.kowalska@example.com>", name="Ola"),
   T("draft_email", "Opened a draft to ola.kowalska@example.com in Convey.", to="ola.kowalska@example.com", subject="Zdjęcia", body="Cześć Ola, przesyłam trzy ostatnie zdjęcia."),
   answer="The draft to Ola is open in Convey; attach IMG_5510–5512 from Pobrane/zdjęcia z telefonu and send.")
ex("posprzątaj pobrane: przenieś pdf-y do dokumentów",
   T("list_folder", "~/Pobrane (6 items): faktura-11.pdf  umowa Play abonament.pdf  IMG_2231.HEIC  setup.flatpakref  slides.pptx  instrukcja zmywarki Bosch.pdf", path="Downloads"),
   T("move_file", "Moved ~/Pobrane/faktura-11.pdf to ~/Dokumenty/faktura-11.pdf.", **{"from": "Downloads/faktura-11.pdf", "to": "Documents"}),
   T("move_file", "Moved ~/Pobrane/umowa Play abonament.pdf to ~/Dokumenty/umowa Play abonament.pdf.", **{"from": "Downloads/umowa Play abonament.pdf", "to": "Documents"}),
   T("move_file", "Moved ~/Pobrane/instrukcja zmywarki Bosch.pdf to ~/Dokumenty/instrukcja zmywarki Bosch.pdf.", **{"from": "Downloads/instrukcja zmywarki Bosch.pdf", "to": "Documents"}),
   answer="Przeniosłem 3 PDF-y do Dokumentów: fakturę, umowę Play i instrukcję zmywarki.")
ex("make a note of what I need for the trip: passport, charger, tickets",
   T("write_file", "Wrote ~/Dokumenty/trip.txt (3 lines).", path="Documents/trip.txt", content="- passport\n- charger\n- tickets\n"),
   answer="Saved Documents/trip.txt: passport, charger, tickets.")
ex("dopisz do notatki z wyjazdu jeszcze krem do opalania",
   T("read_file", "~/Dokumenty/trip.txt (3 lines):\n- passport\n- charger\n- tickets", path="Documents/trip.txt"),
   T("edit_file", "Edited ~/Dokumenty/trip.txt: 1 replacement.", path="Documents/trip.txt", old_text="- tickets\n", new_text="- tickets\n- sunscreen\n"),
   answer="Dopisałem krem do opalania do trip.txt.")

# ------------------------------------------------------------ the web, read properly
ex("how do I reset the fan on a Framework laptop",
   T("web_search", "Results from DuckDuckGo for 'Framework laptop reset fan': 1. Fan control - Framework Knowledge Base - knowledgebase.frame.work/fan …", query="Framework laptop reset fan"),
   T("read_web_page", "Fan control\nhttps://knowledgebase.frame.work/fan\n\nThe fan is controlled by the EC. To reset it, shut the laptop down, unplug the charger, hold the power button for 10 seconds, then start it again…", url="https://knowledgebase.frame.work/fan"),
   answer="Shut it down, unplug the charger, hold the power button for 10 seconds, then start it again (Framework Knowledge Base).")
ex("jak zmienić skrót klawiszowy w GNOME?",
   T("web_search", "Results from DuckDuckGo for 'GNOME zmiana skrótu klawiszowego': 1. Ustawienia → Klawiatura → Skróty klawiszowe - help.gnome.org …", query="GNOME zmiana skrótu klawiszowego"),
   answer="Ustawienia → Klawiatura → Skróty klawiszowe: wybierz skrót i naciśnij nową kombinację (help.gnome.org).")

# ------------------------------------------------------------ the computer
ex("why is my laptop slow",
   T("get_system_info", "Framework Laptop 13, Ryzen AI 5 340, 16 GB (1.2 GB free), Fedora Silverblue 45, up 9 days"),
   T("run_command", "  PID %MEM COMMAND\n 8812 31.4 firefox\n 9120 12.0 code\n 4410  9.8 discord", command="ps -eo pid,%mem,comm --sort=-%mem | head -4"),
   answer="Memory is nearly full (1.2 GB free of 16): Firefox uses 31%, then VS Code and Discord. Closing some tabs or Discord would help; a restart after 9 days wouldn't hurt either.")
ex("czy mam aktualizacje?",
   T("run_command", "Staged: 45.20261116.0 (in 2 packages: firefox, mesa)", command="rpm-ostree status"),
   answer="Tak, nowy obraz (45.20261116.0, m.in. Firefox i Mesa) czeka na restart.")
ex("what's using my battery",
   T("get_battery", "Battery 38%, discharging at 14 W, about 2 h 20 min left."),
   answer="It's draining at 14 W, so about 2 h 20 min left at 38%. Power-saver mode would stretch that.")

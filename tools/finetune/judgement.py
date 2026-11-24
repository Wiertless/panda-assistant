"""Hand-written conversations for judgement: on-screen context used only when the
question points at it, asking back only when it's really unclear, recovering
from errors and declined actions, commands and research, and follow-ups.
Written by Claude (2026-11-24).
"""

from data_dsl import T, chat, ex

END = "\n[/On screen]\n\n"


def screen(app, title, selected=None):
    sel = f" Selected text: «{selected}»" if selected else ""
    return f"[On screen: {app} — “{title}”.{sel}]{END}"


# ------------------------------------------------------------ the selection, when the question points at it
ex(screen("Text Editor", "list.txt", "teh meeting is tomorow at 10, plese bring the slides") + "fix the spelling in this",
   T("rewrite_selection", "Rewrote the selection; the new text is on the clipboard (Ctrl+V to paste).", instruction="fix the spelling",
     text="teh meeting is tomorow at 10, plese bring the slides"),
   answer="Fixed: “The meeting is tomorrow at 10, please bring the slides.” It's on the clipboard.")
ex(screen("Firefox", "Gmail", "Dear Sir, I am writing to complain about the delay of my order number 4471.") + "make it sound friendlier",
   T("rewrite_selection", "Rewrote the selection; the new text is on the clipboard (Ctrl+V to paste).", instruction="make it friendlier",
     text="Dear Sir, I am writing to complain about the delay of my order number 4471."),
   answer="Done, a friendlier version is on the clipboard (Ctrl+V).")
ex(screen("LibreOffice Writer", "raport.odt", "W wyniku przeprowadzonej analizy stwierdzono, iż koszty wzrosły.") + "skróć to zdanie",
   T("rewrite_selection", "Rewrote the selection; the new text is on the clipboard (Ctrl+V to paste).", instruction="skróć",
     text="W wyniku przeprowadzonej analizy stwierdzono, iż koszty wzrosły."),
   answer="Skrócone („Analiza pokazała wzrost kosztów.”), jest w schowku.")
ex(screen("Ptyxis", "kuba@laptop: ~/Projekty", "error[E0382]: borrow of moved value: `cfg`") + "what does this error mean",
   answer="Rust says you used `cfg` after moving it somewhere else, for example into a function or a closure. Pass a reference (`&cfg`) or call `.clone()` before the move.")
chat(screen("Firefox", "Wikipedia — Toruń", "Toruń is a city in northern Poland on the Vistula River.") + "przetłumacz to na polski",
     "Toruń to miasto w północnej Polsce nad Wisłą.")
ex(screen("Firefox", "Stack Overflow", "git reset --hard HEAD~1") + "is this command dangerous?",
   answer="Yes: it deletes your last commit and every uncommitted change for good. `git reset --soft HEAD~1` undoes the commit but keeps the changes.")
# ... and not when the question is about something else
ex(screen("Firefox", "GitHub — kinect-hello", "https://github.com/JeyKul/kinect-hello") + "set a timer for 15 minutes",
   T("set_reminder", "Timer set for 15 minutes.", in_minutes=15))
ex(screen("Text Editor", "notes.txt", "kupić mleko") + "jaka jutro pogoda w Toruniu?",
   T("get_weather", "Toruń now: 3°C, cloudy.\nTomorrow: 1–6°C, sunny.", place="Toruń"),
   answer="Jutro w Toruniu słonecznie, 1–6°C.")
ex(screen("Ptyxis", "kuba@laptop: ~", "sudo dnf upgrade") + "turn on dark mode",
   T("set_dark_mode", "Dark mode on.", enabled=True))
ex(screen("Firefox", "YouTube — lofi beats") + "zatrzymaj to",
   T("media_control", "Paused Firefox (YouTube).", action="pause"), answer="Zatrzymałem.")

# ------------------------------------------------------------ ask back, only when it's really unclear
chat("hook up my headphones",
     "Which ones: the AirPods Pro or the Sony WH-1000XM5?")
ex("connect the sony ones", T("connect_bluetooth", "Connected to Sony WH-1000XM5.", device="Sony WH-1000XM5"),
   before=[("hook up my headphones", "Which ones: the AirPods Pro or the Sony WH-1000XM5?")])
chat("wyślij to na telefon", "Co mam wysłać? Plik, link czy tekst?")
chat("remind me later", "What should I remind you about, and when?")
chat("przypomnij mi", "O czym i kiedy mam ci przypomnieć?")
chat("send the photo", "Which photo? Tell me its name or folder, or say “the newest one”.")
ex("the newest one in Obrazy",
   T("list_folder", "~/Obrazy (newest first): IMG_3012.jpg (3.1 MB, 2026-11-15)  IMG_3011.jpg (2.9 MB, 2026-11-15)  …", path="Obrazy"),
   T("send_file", "Sent IMG_3012.jpg to jakubs-iphone with Taildrop.", device="jakubs-iphone", path="~/Obrazy/IMG_3012.jpg"),
   answer="Sent IMG_3012.jpg to your iPhone.",
   before=[("send the photo to my phone", "Which photo? Tell me its name or folder, or say “the newest one”.")])
# not unclear: the context answers it
ex("send it to my phone",
   T("send_file", "Sent faktura-11.pdf to jakubs-iphone with Taildrop.", device="jakubs-iphone", path="~/Pobrane/faktura-11.pdf"),
   answer="Sent the invoice to your iPhone.",
   before=[("find the november invoice", "Found it: ~/Pobrane/faktura-11.pdf.")])
ex("pair up with my airpods", T("connect_bluetooth", "Connected to Jakub's AirPods Pro.", device="Jakub's AirPods Pro"))

# ------------------------------------------------------------ errors, fixed or reported
ex("open the settings for sound",
   T("open_settings", "Error: no settings panel called sound settings. Panels: wifi, bluetooth, display, sound, power, keyboard, mouse…", panel="sound settings"),
   T("open_settings", "Opened Settings → Sound.", panel="sound"),
   answer="Opened Settings → Sound.")
ex("play the album Discovery",
   T("play_music", "Error (retrying won't help): no song, album or artist matching “Discovery” in your music library", query="Discovery"),
   answer="Discovery isn't in your music library; I only play what's in your Music folder.")
ex("puść coś Queen",
   T("play_music", "Error (retrying won't help): no music found in your library (the Music folder is empty or not indexed)", query="Queen"),
   answer="Nie znalazłem żadnej muzyki w twoim folderze Muzyka.")
ex("delete old-backup.zip from downloads",
   T("trash_file", "The user declined this action.", path="Downloads/old-backup.zip"),
   answer="Okay, I left old-backup.zip where it is.")
ex("usuń wszystkie zrzuty ekranu z pulpitu",
   T("trash_file", "The user declined this action.", path="Desktop/Zrzut ekranu.png"),
   answer="Dobrze, niczego nie usunąłem.")
ex("get my cv onto my phone",
   T("find_files", "~/Dokumenty/CV Jakub 2026.pdf (310 KB, 2026-10-02)", name="cv"),
   T("send_file", "Error (retrying won't help): jakubs-iphone is offline (last seen 3 hours ago).", device="jakubs-iphone", path="~/Dokumenty/CV Jakub 2026.pdf"),
   answer="Your iPhone is offline (last seen 3 hours ago), so the CV didn't go. Try again when it's online.")
ex("what's the weather in Torun",
   T("get_weather", "Toruń now: 2°C, fog.\nTomorrow: 0–5°C, cloudy.", place="Torun"),
   answer="2°C and foggy in Toruń.")

# ------------------------------------------------------------ commands, when asked for one
ex("what's the git status of my panda project",
   T("run_command", "On branch main\nYour branch is up to date with 'origin/main'.\n\nnothing to commit, working tree clean", command="git status", folder="~/Projekty/panda-assistant"),
   answer="Clean: on main, up to date with origin, nothing to commit.")
ex("how much disk space is left",
   T("run_command", "Filesystem  Size  Used Avail Use% Mounted on\n/dev/mapper/luks  475G  263G  210G  56% /var/home", command="df -h ~"),
   answer="210 GB free of 475 GB (56% used).")
ex("ile miejsca zajmuje folder Pobrane?",
   T("run_command", "4.7G\t/var/home/kuba/Pobrane", command="du -sh ~/Pobrane"),
   answer="Pobrane zajmują 4,7 GB.")

# ------------------------------------------------------------ research, the long way
ex("research which robot vacuum is best for pet hair, save it as pdf",
   T("research", "Answer for the user (shown as written): For pet hair, rubber brush rolls and strong suction matter most…\nSaved as ~/Dokumenty/Research/robot vacuum pet hair.pdf",
     question="which robot vacuum is best for pet hair", save_as="pdf"),
   answer=None)
ex("zbadaj, czy warto kupić pompę ciepła do domu w Polsce",
   T("research", "Answer for the user (shown as written): Pompa ciepła opłaca się najbardziej w dobrze ocieplonym domu…",
     question="czy warto kupić pompę ciepła do domu w Polsce"),
   answer=None)
ex("quick research on whether coffee dehydrates you",
   T("research", "Answer for the user (shown as written): Not really: in normal amounts coffee hydrates about as well as water…",
     question="does coffee dehydrate you"),
   answer=None)

# ------------------------------------------------------------ follow-ups that lean on the turn before
ex("a teraz jutro?",
   T("get_weather", "Gdańsk now: 6°C, light rain.\nTomorrow: 4–8°C, rain.", place="Gdańsk"),
   answer="Jutro w Gdańsku 4–8°C i deszcz.",
   before=[("jaka pogoda w Gdańsku?", "W Gdańsku 6°C i lekki deszcz.")])
ex("and in Berlin?",
   T("get_weather", "Berlin now: 9°C, cloudy.\nTomorrow: 7–11°C, cloudy.", place="Berlin"),
   answer="9°C and cloudy in Berlin.",
   before=[("weather in Prague", "11°C and sunny in Prague.")])
ex("make it 20 minutes instead",
   T("cancel_reminder", "Cancelled: timer (22:30).", which="timer"),
   T("set_reminder", "Timer set for 20 minutes.", in_minutes=20),
   answer="Changed it to 20 minutes.",
   before=[("10 minute timer please", "Timer set for 10 minutes.")])
ex("i wyślij to Oli mailem",
   T("find_contact", "Ola Kowalska <ola.kowalska@example.com>", name="Ola"),
   T("draft_email", "Opened a draft to ola.kowalska@example.com in Convey.", to="ola.kowalska@example.com", subject="Lista zakupów", body="Cześć Ola, lista: mleko, jajka, chleb."),
   answer="Szkic do Oli jest otwarty w Convey; sprawdź i wyślij.",
   before=[("zrób listę zakupów: mleko, jajka, chleb", "Zapisałem listę w Dokumenty/lista zakupów.txt.")])
ex("who was the artist again?",
   answer="Dawid Podsiadło.",
   before=[("who sings Lewitacja", "It's by Dawid Podsiadło.")])
ex("dzięki, a jaką mam teraz głośność?",
   T("get_status", "Volume 55%, not muted. Dark mode on. Wi-Fi: Home 5G."),
   answer="Głośność jest na 55%.",
   before=[("ścisz trochę", "Ściszyłem do 55%.")])

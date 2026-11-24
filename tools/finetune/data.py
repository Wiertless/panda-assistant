"""Training conversations for fine-tuning Gemma 4 E2B on Panda's harness.

Written by hand (by Claude, 2026-11-24), in the exact shape the app sends: the
system prompt and tool schemas captured in template.json, "(Answer in X.)" on
the user's message, a tool call required on the first round (chit-chat goes
through the reply tool), tool results, then the answer. Every example is
checked: tools exist, required arguments are there, enums are valid, and no
user message is one of the eval's prompts (the eval must stay unseen).

    python3 -I data.py out/train.jsonl       # writes {"tools": [...], "messages": [...]} per line
"""

import json
import random
import sys
from pathlib import Path

HERE = Path(__file__).parent
TEMPLATE = json.loads((HERE / "template.json").read_text())
TOOLS = {t["function"]["name"]: t for t in TEMPLATE["tools"]}
ORDER = [t["function"]["name"] for t in TEMPLATE["tools"]]
POLISH = set("ąćęłńóśźżĄĆĘŁŃÓŚŹŻ")
PL_WORDS = {"ile", "jak", "czy", "co", "gdzie", "kiedy", "jest", "nie", "mi", "mnie", "sie", "prosze",
            "dzieki", "zrob", "pokaz", "otworz", "wlacz", "wylacz", "ustaw", "jutro", "dzis", "teraz",
            "troche", "ciszej", "glosniej", "napisz", "przypomnij", "znajdz", "wyslij", "puść", "pusc"}
DISTRACTORS = 20


def lang_of(text):
    if any(c in POLISH for c in text):
        return "Polish"
    words = {w.strip(".,?!").lower() for w in text.split()}
    return "Polish" if words & PL_WORDS else "English"


class T:
    """One tool call and what it returned."""

    def __init__(self, tool, result, **args):
        self.tool, self.result, self.args = tool, result, args


EXAMPLES = []


def ex(user, *steps, answer=None, before=(), lang=None):
    """A turn: the user's words, the tool steps in order, the answer after them
    (None for a one-action request the app answers with the tool's own
    confirmation), and earlier turns as (user, assistant) pairs."""
    EXAMPLES.append(dict(user=user, steps=steps, answer=answer, before=before, lang=lang))


def chat(user, answer, before=(), lang=None):
    """Small talk or general knowledge: the answer goes through the reply tool."""
    EXAMPLES.append(dict(user=user, steps=(), answer=answer, before=before, lang=lang, reply=True))


# ---------------------------------------------------------------- settings
ex("switch to dark mode please", T("set_dark_mode", "Dark mode on.", enabled=True))
ex("make it light again", T("set_dark_mode", "Dark mode off.", enabled=False))
ex("wyłącz ciemny motyw", T("set_dark_mode", "Dark mode off.", enabled=False), answer="Wyłączyłem ciemny motyw.")
ex("ściemnij ekran trochę", T("set_brightness", "Brightness set to 40%.", change="down"), answer="Przyciemniłem ekran do 40%.")
ex("screen's too dark", T("set_brightness", "Brightness set to 80%.", change="up"))
ex("brightness to 30", T("set_brightness", "Brightness set to 30%.", percent=30))
ex("turn the sound down a little", T("set_volume", "Volume set to 45%.", change="down"))
ex("podgłośnij muzykę", T("set_volume", "Volume set to 75%.", change="up"), answer="Podgłośniłem do 75%.")
ex("volume 20 percent", T("set_volume", "Volume set to 20%.", percent=20))
ex("ustaw głośność na połowę", T("set_volume", "Volume set to 50%.", percent=50), answer="Głośność jest na 50%.")
ex("mute", T("set_mute", "Sound muted.", muted=True))
ex("odcisz dźwięk", T("set_mute", "Sound unmuted.", muted=False), answer="Dźwięk jest znowu włączony.")
ex("don't disturb me for a while", T("set_do_not_disturb", "Do Not Disturb on.", enabled=True))
ex("włącz nie przeszkadzać na chwilę", T("set_do_not_disturb", "Do Not Disturb on.", enabled=True), answer="Włączyłem tryb Nie przeszkadzać.")
ex("night light on", T("set_night_light", "Night Light on.", enabled=True))
ex("wyłącz nocne światło", T("set_night_light", "Night Light off.", enabled=False), answer="Nocne światło wyłączone.")
ex("save battery, I'm on the train", T("set_power_mode", "Power mode: power-saver.", mode="power-saver"))
ex("tryb wydajności", T("set_power_mode", "Power mode: performance.", mode="performance"), answer="Przełączyłem na tryb wydajności.")
ex("kill the wifi for now", T("set_wifi", "Wi-Fi off.", enabled=False))
ex("włącz bluetooth", T("set_bluetooth", "Bluetooth on.", enabled=True), answer="Bluetooth włączony.")
ex("connect my sony headphones",
   T("connect_bluetooth", "Connected to Sony WH-1000XM5.", device="Sony WH-1000XM5"))
ex("rozłącz słuchawki airpods",
   T("disconnect_bluetooth", "Disconnected Jakub's AirPods Pro.", device="Jakub's AirPods Pro"),
   answer="Rozłączyłem AirPodsy.")
ex("join the office guest network",
   T("connect_network", "Connected to Biuro-Gosc.", name="Biuro-Gosc"))
ex("I can barely see the cursor, make it larger",
   T("find_settings", "org.gnome.desktop.interface cursor-size (integer, now 24): pointer size", query="cursor size"),
   T("change_setting", "cursor-size set to 48.", schema="org.gnome.desktop.interface", key="cursor-size", value="48"),
   answer="Made the pointer bigger (48).")
ex("chcę widzieć procent baterii obok ikonki",
   T("find_settings", "org.gnome.desktop.interface show-battery-percentage (boolean, now false): battery percentage in the top bar", query="battery percentage"),
   T("change_setting", "show-battery-percentage set to true.", schema="org.gnome.desktop.interface", key="show-battery-percentage", value="true"),
   answer="Procent baterii jest teraz widoczny na górnym pasku.")
ex("open the display settings", T("open_settings", "Opened Settings → Displays.", panel="display"))

# ------------------------------------------------------------ quick facts
ex("how much battery do I have", T("get_battery", "Battery 64%, discharging, about 3 h 40 min left."),
   answer="64%, about 3 h 40 min left.")
ex("na ile starczy mi baterii?", T("get_battery", "Battery 23%, discharging, about 1 h 5 min left."),
   answer="23%, zostało około godziny. Może warto podłączyć ładowarkę.")
ex("what day is it", T("get_datetime", "Monday 2026-11-16 09:12 CET"), answer="It's Monday, 16 November.")
ex("jaka jest teraz godzina?", T("get_datetime", "Sunday 2026-11-15 22:40 CET"), answer="Jest 22:40.")
ex("what's 23% of 1840", T("calculate", "23% * 1840 = 423.2", expression="23% * 1840"), answer="423.2")
ex("policz 12 razy 37", T("calculate", "12 * 37 = 444", expression="12 * 37"), answer="12 × 37 = 444.")
ex("split 186.40 between 4 people", T("calculate", "186.40 / 4 = 46.6", expression="186.40 / 4"),
   answer="46.60 each.")
ex("ile to 8 procent z 250 zł?", T("calculate", "8% * 250 = 20", expression="8% * 250"), answer="To 20 zł.")
ex("convert 72 fahrenheit to celsius", T("calculate", "(72 - 32) * 5 / 9 = 22.222222222222", expression="(72 - 32) * 5 / 9"),
   answer="About 22.2 °C.")
ex("jaka pogoda w Gdańsku?",
   T("get_weather", "Gdańsk now: 6°C, light rain, wind 24 km/h.\nTomorrow: 4–8°C, rain.\nWednesday: 3–7°C, cloudy.", place="Gdańsk"),
   answer="W Gdańsku 6°C i lekki deszcz; jutro też deszczowo, 4–8°C.")
ex("will I need an umbrella in Kraków tomorrow",
   T("get_weather", "Kraków now: 9°C, cloudy.\nTomorrow: 7–12°C, showers in the afternoon.\nWednesday: 5–10°C, sunny.", place="Kraków"),
   answer="Yes, showers are expected tomorrow afternoon (7–12°C).")
ex("what's on today",
   T("get_events", "Today, Monday 2026-11-16:\n10:00–11:00 Stand-up\n14:30–15:00 Dentist", when="today"),
   answer="Stand-up at 10:00 and the dentist at 14:30.")
ex("czy mam coś w piątek?", T("get_events", "Friday 2026-11-20: nothing.", when="friday"),
   answer="W piątek nic nie masz w kalendarzu.")

# ------------------------------------------------------------ calendar, reminders
ex("put the dentist on thursday at 3pm in my calendar",
   T("create_event", "Added “Dentist” on Thursday 2026-11-19 15:00–16:00 to Personal.", title="Dentist", start="2026-11-19 15:00"),
   answer="Added the dentist on Thursday at 15:00.")
ex("dodaj spotkanie z Olą jutro o 12",
   T("create_event", "Added “Spotkanie z Olą” on Monday 2026-11-16 12:00–13:00 to Personal.", title="Spotkanie z Olą", start="2026-11-16 12:00"),
   answer="Dodałem spotkanie z Olą na jutro na 12:00.")
ex("remind me in half an hour to check the oven",
   T("set_reminder", "I'll remind you at 22:40: check the oven.", in_minutes=30, text="check the oven"))
ex("przypomnij mi jutro o 8 o wyniesieniu śmieci",
   T("set_reminder", "I'll remind you on Monday at 08:00: wynieść śmieci.", at="2026-11-16 08:00", text="wynieść śmieci"),
   answer="Przypomnę ci jutro o 8:00 o śmieciach.")
ex("10 minute timer", T("set_reminder", "Timer set for 10 minutes (22:20).", in_minutes=10))
ex("what reminders do I have",
   T("list_reminders", "1. 22:40 today: check the oven\n2. Monday 08:00: wynieść śmieci"),
   answer="Two: check the oven at 22:40, and the rubbish tomorrow at 8:00.")
ex("cancel the oven reminder", T("cancel_reminder", "Cancelled: check the oven (22:40).", which="oven"))

# ------------------------------------------------------------ apps, links
ex("start firefox for me", T("open_app", "Opened Firefox.", name="Firefox"))
ex("uruchom kalkulator", T("open_app", "Opened Calculator.", name="calculator"), answer="Otworzyłem kalkulator.")
ex("open my email app", T("open_app", "Opened Convey.", name="email"))
ex("go to github.com", T("open_url", "Opened https://github.com in Firefox.", url="https://github.com"))
ex("I need a terminal in Projekty",
   T("open_terminal", "Opened a terminal in ~/Projekty.", folder="~/Projekty"))
ex("play some Daft Punk", T("play_music", "Playing 14 songs (Daft Punk) in Gapless.", query="Daft Punk"))
ex("puść coś od Dawida Podsiadło",
   T("play_music", "Playing 11 songs (Dawid Podsiadło) in Gapless.", query="Dawid Podsiadło"),
   answer="Puszczam Dawida Podsiadło w Gapless.")
ex("skip this track", T("media_control", "Next track in Gapless: Lewitacja.", action="next_track"))
ex("zapauzuj muzykę", T("media_control", "Paused Gapless.", action="pause"), answer="Zatrzymałem muzykę.")
ex("what song is this", T("media_control", "Gapless is playing “Get Lucky” by Daft Punk (Random Access Memories).", action="status"),
   answer="“Get Lucky” by Daft Punk.")

# ------------------------------------------------------------ files
ex("find my rental agreement",
   T("find_files", "~/Dokumenty/umowa najmu 2026.pdf (412 KB, 2026-02-01)\n~/Pobrane/umowa najmu skan.pdf (2.1 MB, 2026-01-28)", name="umowa najmu"),
   answer="Found two: ~/Dokumenty/umowa najmu 2026.pdf and a scan in Downloads, umowa najmu skan.pdf.")
ex("what's in my downloads folder",
   T("list_folder", "~/Pobrane (5 items): faktura-11.pdf (88 KB, 2026-11-14)  IMG_2231.HEIC (2.9 MB, 2026-11-14)  setup.flatpakref (1 KB, 2026-11-10)  slides.pptx (6.3 MB, 2026-11-09)  notes.txt (2 KB, 2026-11-02)", path="Downloads"),
   answer="Five files: an invoice (faktura-11.pdf), a photo IMG_2231.HEIC, slides.pptx, notes.txt and a flatpakref.")
ex("make a shopping list file with milk, eggs and bread",
   T("write_file", "Wrote ~/Dokumenty/shopping list.txt (3 lines).", path="Documents/shopping list.txt", content="- milk\n- eggs\n- bread\n"),
   answer="Saved Documents/shopping list.txt with milk, eggs and bread.")
ex("zapisz notatkę: zadzwonić do mechanika w środę",
   T("write_file", "Wrote ~/Dokumenty/notatka.txt (1 line).", path="Documents/notatka.txt", content="Zadzwonić do mechanika w środę.\n"),
   answer="Zapisałem notatkę w Dokumenty/notatka.txt.")
ex("rename notes.txt in downloads to meeting notes.txt",
   T("move_file", "Moved ~/Pobrane/notes.txt to ~/Pobrane/meeting notes.txt.", **{"from": "Downloads/notes.txt", "to": "meeting notes.txt"}),
   answer="Renamed it to meeting notes.txt.")
ex("delete setup.flatpakref from downloads",
   T("trash_file", "Moved ~/Pobrane/setup.flatpakref to the Trash.", path="Downloads/setup.flatpakref"),
   answer="Moved setup.flatpakref to the Trash (Undo puts it back).")
ex("what does notes.txt in my documents say",
   T("read_file", "~/Dokumenty/notes.txt (4 lines):\nCall the landlord about the heater.\nBuy a HDMI cable.\nBook train to Kraków.\nRenew the gym pass.", path="Documents/notes.txt"),
   answer="Four things: call the landlord about the heater, buy an HDMI cable, book the train to Kraków, renew the gym pass.")
ex("change 'Kraków' to 'Gdańsk' in my notes",
   T("edit_file", "Edited ~/Dokumenty/notes.txt: 1 replacement.", path="Documents/notes.txt", old_text="Book train to Kraków.", new_text="Book train to Gdańsk."),
   answer="Changed it: the note now says Book train to Gdańsk.")
ex("jaką kaucję wpłaciłem według umowy najmu?",
   T("find_files", "~/Dokumenty/umowa najmu 2026.pdf (412 KB, 2026-02-01)", name="umowa najmu"),
   T("ask_document", "From ~/Dokumenty/umowa najmu 2026.pdf: § 5. Najemca wpłaca kaucję w wysokości 3 600 zł, zwrotną w ciągu 30 dni od zakończenia najmu.",
     path="~/Dokumenty/umowa najmu 2026.pdf", question="ile wynosi kaucja?"),
   answer="Kaucja wynosi 3 600 zł i jest zwracana w ciągu 30 dni po końcu najmu.")

# ------------------------------------------------------------ pictures
ex("shrink the photos in Obrazy/Wakacje so I can email them",
   T("edit_image", "Saved 24 of 24 in ~/Obrazy/Edited (the originals are unchanged):\n~/Obrazy/Edited/IMG_1001.jpg: 4.2 MB 4032×3024 → 0.7 MB 1920×1440\n…",
     files="Obrazy/Wakacje", resize="1920"),
   answer="Done: 24 smaller copies (about 0.7 MB each) in Obrazy/Edited; the originals are untouched.")
ex("zamień IMG_2231.HEIC z pobranych na jpg",
   T("edit_image", "Saved 1 of 1 in ~/Obrazy/Edited (the originals are unchanged):\n~/Obrazy/Edited/IMG_2231.jpg: 2.9 MB 4032×3024 → 1.8 MB 4032×3024",
     files="Downloads/IMG_2231.HEIC", format="jpg"),
   answer="Gotowe: Obrazy/Edited/IMG_2231.jpg.")
ex("remove the location from the photos I'm about to post, they're in Obrazy/Do wrzucenia",
   T("edit_image", "Saved 6 of 6 in ~/Obrazy/Edited (the originals are unchanged):\n~/Obrazy/Edited/a.jpg: 3.1 MB 4000×3000 → 3.0 MB 4000×3000\n…",
     files="Obrazy/Do wrzucenia", remove_location=True),
   answer="Done: 6 copies without location or camera details in Obrazy/Edited.")
ex("rotate scan.jpg on my desktop 90 degrees",
   T("edit_image", "Saved 1 of 1 in ~/Obrazy/Edited (the originals are unchanged):\n~/Obrazy/Edited/scan.jpg: 1.2 MB 2480×3508 → 1.2 MB 3508×2480",
     files="Desktop/scan.jpg", rotate=90),
   answer="Rotated copy: Obrazy/Edited/scan.jpg.")
ex("zrób jeden pdf ze skanów w Pobrane/skany i nazwij go faktury",
   T("images_to_pdf", "Made ~/Obrazy/Edited/faktury.pdf from 5 pictures (3.4 MB)", files="Downloads/skany", name="faktury"),
   answer="Zrobiłem faktury.pdf z 5 skanów (Obrazy/Edited).")

# ------------------------------------------------------------ devices
ex("send the invoice from downloads to my phone",
   T("find_files", "~/Pobrane/faktura-11.pdf (88 KB, 2026-11-14)", name="faktura", folder="Downloads"),
   T("send_file", "Sent faktura-11.pdf to jakubs-iphone with Taildrop.", device="jakubs-iphone", path="~/Pobrane/faktura-11.pdf"),
   answer="Sent faktura-11.pdf to your iPhone.")
ex("wyślij mi na telefon adres: ul. Długa 5, Toruń",
   T("send_text", "Sent the note to jakubs-iphone with Taildrop.", device="jakubs-iphone", text="ul. Długa 5, Toruń", title="Adres"),
   answer="Wysłałem adres na iPhone'a.")
ex("send this screenshot to my phone",
   T("save_screenshot", "Saved the screenshot as ~/Obrazy/Zrzuty ekranu/Screenshot From 2026-11-15 22-10-04.png. To send it, pass exactly that path to send_file."),
   T("send_file", "Sent Screenshot From 2026-11-15 22-10-04.png to jakubs-iphone with Taildrop.", device="jakubs-iphone",
     path="~/Obrazy/Zrzuty ekranu/Screenshot From 2026-11-15 22-10-04.png"),
   answer="Took a screenshot and sent it to your iPhone.")
# recovering from a wrong device name
ex("send notes.txt from documents to my iphone",
   T("send_file", "Error: no device called iphone. Devices: jakubs-iphone (iOS, online), framework (linux, this computer).", device="iphone", path="Documents/notes.txt"),
   T("send_file", "Sent notes.txt to jakubs-iphone with Taildrop.", device="jakubs-iphone", path="Documents/notes.txt"),
   answer="Sent notes.txt to your iPhone.")

# ------------------------------------------------------------ found, not taken
ex("find the screenshot I took yesterday and send it to my phone",
   T("find_files", "~/Obrazy/Zrzuty ekranu/Screenshot From 2026-11-14 17-02-51.png (640 KB, 2026-11-14)\n~/Obrazy/Zrzuty ekranu/Screenshot From 2026-11-10 09-30-12.png (512 KB, 2026-11-10)",
     name="Screenshot", folder="Obrazy"),
   T("send_file", "Sent Screenshot From 2026-11-14 17-02-51.png to jakubs-iphone with Taildrop.", device="jakubs-iphone",
     path="~/Obrazy/Zrzuty ekranu/Screenshot From 2026-11-14 17-02-51.png"),
   answer="Sent yesterday's screenshot (17:02) to your iPhone.")
ex("znajdź zdjęcie paragonu z poniedziałku i zrób z niego pdf",
   T("find_files", "~/Obrazy/paragon media expert.jpg (1.9 MB, 2026-11-09)", name="paragon"),
   T("images_to_pdf", "Made ~/Obrazy/Edited/paragon.pdf from 1 pictures (420 KB)", files="~/Obrazy/paragon media expert.jpg", name="paragon"),
   answer="Zrobiłem paragon.pdf ze zdjęcia z poniedziałku (Obrazy/Edited).")

# ------------------------------------------------------------ music: look up, then act
ex("who sings Lewitacja, and send me the song",
   T("find_files", "~/Muzyka/Dawid Podsiadło/Lewitacja.flac (28 MB, 2025-03-02)", name="Lewitacja", folder="Music"),
   T("send_file", "Sent Lewitacja.flac to jakubs-iphone with Taildrop.", device="jakubs-iphone", path="~/Muzyka/Dawid Podsiadło/Lewitacja.flac"),
   answer="It's by Dawid Podsiadło; sent the song to your iPhone.")

# ------------------------------------------------------------ mail
ex("did the landlord write back",
   T("search_mail", "Mail matching “landlord”, newest first:\n[home:212] Marek Nowak — Re: heater (Saturday 2026-11-14 18:20, the newest)\n  Hi, the technician can come on Tuesday between 10 and 12.\nread_mail with an id in brackets reads the whole message.", query="landlord"),
   answer="Yes, on Saturday: the technician can come on Tuesday between 10 and 12.")
ex("co nowego na poczcie?",
   T("search_mail", "Mail matching “”, newest first:\n[uni:88] USOS — Nowa ocena (Monday 2026-11-16 08:05, the newest)\n  Wystawiono ocenę z przedmiotu Analiza matematyczna.\n[home:213] Allegro — Twoja paczka jest w drodze (Sunday 2026-11-15 19:40)\n  Przesyłka dotrze jutro.", query=""),
   answer="Najnowszy: USOS, nowa ocena z analizy (dziś 8:05). Wcześniej Allegro: paczka będzie jutro.")
ex("write Ola an email that I'm running 15 minutes late",
   T("find_contact", "Ola Kowalska <ola.kowalska@example.com>", name="Ola"),
   T("draft_email", "Opened a draft to ola.kowalska@example.com in Convey.", to="ola.kowalska@example.com", subject="Running late", body="Hi Ola, I'll be about 15 minutes late. Sorry!"),
   answer="The draft to Ola is open in Convey; check it and press Send.")

# ------------------------------------------------------------ web
ex("what's the latest Firefox version",
   T("web_search", "Results from DuckDuckGo for 'latest firefox version': 1. Firefox 147.0 release notes - mozilla.org - Released November 11, 2026…", query="latest firefox version"),
   answer="Firefox 147.0, released on 11 November 2026 (mozilla.org).")
ex("kto wygrał wczoraj mecz Polska Holandia?",
   T("web_search", "Results from DuckDuckGo for 'Polska Holandia mecz wynik': 1. Polska 1:2 Holandia - sport.pl - Bramkę dla Polski strzelił Lewandowski…", query="Polska Holandia mecz wynik"),
   answer="Holandia wygrała 2:1; dla Polski strzelił Lewandowski (sport.pl).")
ex("look into which e-reader is best for reading PDFs",
   T("research", "Answer for the user (shown as written): For PDFs, a large 10.3-inch E Ink screen matters most: …", question="which e-reader is best for reading PDFs"),
   answer=None)

# ------------------------------------------------------------ clipboard, selection, screen
ex("fix the grammar in what I selected",
   T("rewrite_selection", "Rewrote the selection; the new text is on the clipboard (Ctrl+V to paste).", instruction="fix the grammar"),
   answer="Fixed it; it's on the clipboard, paste with Ctrl+V.")
ex("zaznaczony fragment przełóż na angielski",
   T("rewrite_selection", "Rewrote the selection; the new text is on the clipboard (Ctrl+V to paste).", instruction="translate to English"),
   answer="Przetłumaczone; tekst jest w schowku (Ctrl+V).")
ex("what was that link I copied earlier",
   T("clipboard_history", "Recent copies, newest first:\n1. https://bilety.pkp.pl/rezerwacja/88231\n2. ul. Długa 5, Toruń", query="http"),
   answer="https://bilety.pkp.pl/rezerwacja/88231")
ex("copy my address to the clipboard", T("copy_to_clipboard", "Copied to the clipboard.", text="ul. Długa 5, Toruń"),
   answer="Copied: ul. Długa 5, Toruń.")

# ------------------------------------------------------------ memory
ex("remember that I prefer Celsius", T("remember", "Remembered: Prefers Celsius.", fact="Prefers Celsius"))
ex("zapamiętaj, że mój pies ma na imię Burek", T("remember", "Remembered: The user's dog is called Burek.", fact="The user's dog is called Burek"),
   answer="Zapamiętałem: twój pies to Burek.")
ex("forget that I live in Toruń", T("forget", "Forgot: Lives in Toruń.", fact="Lives in Toruń"))

# ------------------------------------------------------------ undo
ex("revert that", T("undo", "Undid “Dark mode on”: dark mode is off again."),
   before=[("switch to dark mode please", "Dark mode on.")])
ex("cofnij to wszystko, co zrobiłeś", T("undo", "Undid “Edit 24 pictures”: removed 24 new files.", whole_task=True),
   answer="Cofnięte: usunąłem 24 nowe pliki.",
   before=[("zmniejsz zdjęcia z Obrazy/Wakacje", "Zrobiłem 24 mniejsze kopie w Obrazy/Edited.")])

# ------------------------------------------------------------ follow-ups
ex("a bit more", T("set_volume", "Volume set to 60%.", change="up"),
   before=[("turn the volume up", "Volume set to 45%.")])
ex("i jeszcze trochę ciszej", T("set_volume", "Volume set to 30%.", change="down"),
   answer="Ściszyłem do 30%.", before=[("ścisz", "Ściszyłem do 45%.")])
ex("and send it to my phone too",
   T("send_file", "Sent shopping list.txt to jakubs-iphone with Taildrop.", device="jakubs-iphone", path="Documents/shopping list.txt"),
   answer="Sent the shopping list to your iPhone.",
   before=[("make a shopping list file with milk and eggs", "Saved Documents/shopping list.txt with milk and eggs.")])

# ------------------------------------------------------------ chat, no tools
chat("thanks a lot", "You're welcome!")
chat("dzięki", "Proszę bardzo!")
chat("cheers, thanks", "Glad to help!", before=[("co nowego na poczcie?", "Najnowszy: USOS, nowa ocena z analizy.")])
chat("hi", "Hi! What can I do for you?")
chat("cześć, jak się masz?", "Dzięki, dobrze! W czym mogę pomóc?")
chat("what's the capital of Australia", "Canberra.")
chat("explain what an NPU is in one sentence",
     "An NPU is a processor built for AI work: it runs neural networks, like me, faster and with less power than the CPU.")
chat("ile to jest 7 razy 6", "42.")
chat("tell me a short joke", "Why do programmers prefer dark mode? Because light attracts bugs.")
chat("dlaczego niebo jest niebieskie?",
     "Bo światło słoneczne rozprasza się w powietrzu, a niebieskie fale (krótsze) rozpraszają się najmocniej: to rozpraszanie Rayleigha.")
chat("think it through: why is the sky blue?",
     "Sunlight scatters off the molecules in the air, and short blue wavelengths scatter the most (Rayleigh scattering), so blue light reaches your eyes from all over the sky.")


# ---------------------------------------------------------------- building
def check(e):
    for s in e["steps"]:
        assert s.tool in TOOLS, f"no tool {s.tool}"
        p = TOOLS[s.tool]["function"]["parameters"]
        props = p.get("properties", {})
        for r in p.get("required", []):
            assert r in s.args, f"{s.tool}: {r} missing ({e['user']})"
        for k, v in s.args.items():
            assert k in props, f"{s.tool}: no argument {k} ({e['user']})"
            if "enum" in props[k]:
                assert v in props[k]["enum"], f"{s.tool}.{k}={v} not in {props[k]['enum']}"


def eval_prompts():
    path = HERE.parent.parent / "core" / "eval" / "scenarios.json"
    out = set()
    for s in json.loads(path.read_text()):
        for p in [s["prompt"], *s.get("paraphrases", [])]:
            out.add(p.strip().lower())
    return out


def call(i, name, args):
    return {"id": f"call_{i}", "type": "function", "function": {"name": name, "arguments": json.dumps(args, ensure_ascii=False)}}


def tools_for(e, rng):
    """The example's own tools, reply and plan, and some others (in the app's order)."""
    need = {s.tool for s in e["steps"]} | {"reply", "plan"}
    others = [n for n in ORDER if n not in need]
    pick = need | set(rng.sample(others, DISTRACTORS))
    return [TOOLS[n] for n in ORDER if n in pick]


def render(e, rng):
    lang = e["lang"] or lang_of(e["user"])
    msgs = [{"role": "system", "content": TEMPLATE["system"]}]
    for u, a in e["before"]:
        msgs.append({"role": "user", "content": u})
        msgs.append({"role": "assistant", "content": a})
    msgs.append({"role": "user", "content": f"{e['user']}\n\n(Answer in {lang}.)"})
    if e.get("reply"):
        msgs.append({"role": "assistant", "content": "", "tool_calls": [call(0, "reply", {"text": e["answer"]})]})
        msgs.append({"role": "tool", "tool_call_id": "call_0", "content": "Shown to the user."})
    else:
        for i, s in enumerate(e["steps"]):
            msgs.append({"role": "assistant", "content": "", "tool_calls": [call(i, s.tool, s.args)]})
            msgs.append({"role": "tool", "tool_call_id": f"call_{i}", "content": s.result})
        if e["answer"]:
            msgs.append({"role": "assistant", "content": e["answer"]})
    return {"tools": tools_for(e, rng), "messages": msgs}


def main(out):
    rng = random.Random(20261124)
    banned = eval_prompts()
    for e in EXAMPLES:
        check(e)
        assert e["user"].strip().lower() not in banned, f"eval prompt in training data: {e['user']}"
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    with open(out, "w") as f:
        for e in EXAMPLES:
            f.write(json.dumps(render(e, rng), ensure_ascii=False) + "\n")
    pl = sum(1 for e in EXAMPLES if (e["lang"] or lang_of(e["user"])) == "Polish")
    print(f"{len(EXAMPLES)} examples ({pl} Polish) -> {out}")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else str(HERE / "out" / "train.jsonl"))

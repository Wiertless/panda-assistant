"""Near-tool choices and many wordings (Claude, 2026-11-24): where two tools look
alike, the request says which (a text or a file to send, a screenshot to keep or
only to look at, a file's name or what's inside it, a quick fact or research, the
window's text or the selection rewritten), and more ways of saying the commonest
requests in both languages.
"""

import random

from data_dsl import T, chat, ex

R = random.Random(41)
PHONE = "jakubs-iphone"

# ------------------------------------------------------------ text or file
for u, text in [("send my address to my phone", "ul. Długa 5, 87-100 Toruń"), ("text the wifi password to my phone", "kot123"),
                ("wyślij na telefon numer do Marka", "Marek Nowak: +48 601 234 567")]:
    ex(u, T("send_text", f"Sent the note to {PHONE} with Taildrop.", device=PHONE, text=text),
       answer="Wysłałem na iPhone'a." if "wyślij" in u else "Sent it to your iPhone.")
for u, path in [("send budget.ods to my phone", "~/Dokumenty/budżet 2026.ods"), ("wyślij plik notes.txt na telefon", "Documents/notes.txt")]:
    ex(u, T("send_file", f"Sent {path.rsplit('/', 1)[1]} to {PHONE} with Taildrop.", device=PHONE, path=path),
       answer="Wysłałem plik na iPhone'a." if "wyślij" in u else "Sent it to your iPhone.")
ex("send that answer to my phone", T("send_text", f"Sent the note to {PHONE} with Taildrop.", device=PHONE, text="Rent: 2,400 zł a month, due by the 10th."),
   answer="Sent it to your iPhone.", before=[("how much is the rent in my lease", "2,400 zł a month, due by the 10th.")])

# ------------------------------------------------------------ a screenshot to keep, or only to look at
ex("save a screenshot", T("save_screenshot", "Saved the screenshot as ~/Obrazy/Zrzuty ekranu/Screenshot From 2026-11-16 12-01-11.png. To send it, pass exactly that path to send_file."),
   answer="Saved it in Obrazy/Zrzuty ekranu.")
ex("zrób zrzut ekranu", T("save_screenshot", "Saved the screenshot as ~/Obrazy/Zrzuty ekranu/Screenshot From 2026-11-16 12-02-40.png. To send it, pass exactly that path to send_file."),
   answer="Zapisałem zrzut w Obrazy/Zrzuty ekranu.")

# ------------------------------------------------------------ a file's name, or what's inside
ex("which of my notes mention the heater",
   T("search_text", "~/Dokumenty/notes.txt:1: Call the landlord about the heater.\n~/Dokumenty/mieszkanie.md:7: grzejnik w sypialni nie grzeje (heater)", query="heater", folder="Documents"),
   answer="Two: notes.txt (call the landlord about the heater) and mieszkanie.md (the bedroom heater).")
ex("w których plikach jest słowo kaucja?",
   T("search_file_contents", "~/Dokumenty/umowa najmu 2026.pdf: … wpłaca kaucję w wysokości 3 600 zł …\n~/Dokumenty/mieszkanie.md: … zwrot kaucji po wyprowadzce …", query="kaucja"),
   answer="W umowie najmu (kaucja 3 600 zł) i w notatce mieszkanie.md.")
ex("find the file called mieszkanie",
   T("find_files", "~/Dokumenty/mieszkanie.md (3 KB, 2026-11-10)", name="mieszkanie"), answer="~/Dokumenty/mieszkanie.md")

# ------------------------------------------------------------ quick fact, or research
ex("when does the Christmas market in Toruń open",
   T("web_search", "Results from DuckDuckGo for 'jarmark bożonarodzeniowy Toruń 2026': 1. Jarmark na Rynku Staromiejskim od 27 listopada do 23 grudnia - torun.pl…", query="jarmark bożonarodzeniowy Toruń 2026"),
   answer="From 27 November to 23 December on the Old Town Square (torun.pl).")
ex("research the best budget mechanical keyboards",
   T("research", "Answer for the user (shown as written): Under 300 zł, hot-swappable boards with gasket mounts give the most for the money…",
     question="the best budget mechanical keyboards"), answer=None)

# ------------------------------------------------------------ the window's text, or the selection rewritten
ex("what's this page about", T("read_window", "Firefox — “Rust 2027 edition announced”\nThe Rust project announced the 2027 edition with async closures stable, …"),
   answer="It's the Rust 2027 edition announcement: async closures become stable, among other things.")
ex("o czym jest ten artykuł?", T("read_window", "Firefox — “NBP obniża stopy procentowe”\nRada Polityki Pieniężnej obniżyła stopę referencyjną o 0,25 pkt proc. do 5,00%…"),
   answer="RPP obniżyła stopę referencyjną o 0,25 pkt proc., do 5,00%.")
ex("rewrite the selected paragraph to be shorter",
   T("rewrite_selection", "Rewrote the selection; the new text is on the clipboard (Ctrl+V to paste).", instruction="make it shorter"),
   answer="Shorter version is on the clipboard (Ctrl+V).")

# ------------------------------------------------------------ the commonest requests, many ways
MANY = [
    ("set_dark_mode", {"enabled": True}, "Dark mode on.", None, "Ciemny motyw włączony.",
     ["dark mode", "go dark mode", "turn on the dark theme please", "I prefer dark, switch it", "night theme on"],
     ["tryb ciemny", "ciemny motyw proszę", "przełącz na tryb nocny ekranu", "daj ciemno"]),
    ("set_volume", {"percent": 0}, "Volume set to 0%.", None, "Głośność na zero.",
     ["volume all the way down", "volume to zero"], ["głośność na zero", "ścisz do zera"]),
    ("set_volume", {"percent": 100}, "Volume set to 100%.", None, "Głośność na maksa.",
     ["max volume", "volume all the way up"], ["głośność na maksa", "podgłośnij na full"]),
    ("set_brightness", {"percent": 100}, "Brightness set to 100%.", None, "Jasność na maksimum.",
     ["full brightness", "brightness max"], ["jasność na maksa", "pełna jasność"]),
    ("media_control", {"action": "play_pause"}, "Toggled Gapless: paused.", None, "Pauza.",
     ["pause or play", "toggle the music"], ["pauza/play", "przełącz odtwarzanie"]),
    ("get_weather", {"place": "Toruń"}, "Toruń now: 1°C, snow.\nTomorrow: -2–3°C, snow.", "1°C and snowing in Toruń.", "W Toruniu 1°C i pada śnieg.",
     ["weather in Toruń", "is it snowing in Toruń", "how's the weather in Torun today"], ["pogoda Toruń", "czy w Toruniu pada śnieg?", "jaka dziś pogoda w Toruniu?"]),
    ("get_events", {"when": "tomorrow"}, "Tomorrow, Tuesday 2026-11-17:\n09:00–10:00 Stand-up\n18:00–19:30 Siłownia",
     "Stand-up at 9:00 and the gym at 18:00.", "Stand-up o 9:00 i siłownia o 18:00.",
     ["what's on tomorrow", "my schedule for tomorrow", "anything tomorrow?"], ["co mam jutro?", "plan na jutro", "jutrzejsze spotkania?"]),
    ("search_mail", {"query": ""}, "Mail matching “”, newest first:\n[home:610] Allegro — Paczka odebrana (Monday 2026-11-16 13:02, the newest)\n  Paczka została odebrana z automatu.",
     "The newest: Allegro says your parcel was picked up.", "Najnowszy: Allegro, paczka odebrana z automatu.",
     ["check my mail", "anything in my inbox", "new emails?"], ["sprawdź pocztę", "mam nowe maile?", "co w skrzynce?"]),
]
for tool, args, res, en_ans, pl_ans, en, pl in MANY:
    for u in en:
        ex(u, T(tool, res, **args), answer=en_ans)
    for u in pl:
        ex(u, T(tool, res, **args), answer=pl_ans)

for q, a in [("thanks, that's all", "You're welcome!"), ("great job", "Thanks!"), ("perfect", "Glad it helped!"),
             ("to wszystko, dzięki", "Proszę bardzo!"), ("super", "Cieszę się!"), ("ok dzięki", "Nie ma sprawy!"),
             ("what's your name", "Panda."), ("jak masz na imię?", "Panda."),
             ("are you better than chatgpt", "I'm smaller, but I run on your laptop, keep your data here and can work with your computer."),
             ("can you hear me", "I read what you type; I can't listen yet.")]:
    chat(q, a)

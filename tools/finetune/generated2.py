"""More generated families (Claude, 2026-11-24): calendar events and dates, mail
searches and reads, lookups on the web, links, notes to the phone, media,
memory, clipboard, picture edits, and more wordings of the everyday requests.
Values are spelled out; Polish forms are written, not inflected by rule.
"""

import random

from data_dsl import T, chat, ex

R = random.Random(11)

DAYS = [("Monday", "poniedziałek", "w poniedziałek", "2026-11-16"), ("Tuesday", "wtorek", "we wtorek", "2026-11-17"),
        ("Wednesday", "środa", "w środę", "2026-11-18"), ("Thursday", "czwartek", "w czwartek", "2026-11-19"),
        ("Friday", "piątek", "w piątek", "2026-11-20"), ("Saturday", "sobota", "w sobotę", "2026-11-21")]

# ------------------------------------------------------------ events
EVENTS = [("dentist", "Dentysta", "dentysta", "dentystę"), ("gym", "Siłownia", "siłownia", "siłownię"),
          ("call with Ola", "Rozmowa z Olą", "rozmowa z Olą", "rozmowę z Olą"),
          ("team lunch", "Lunch zespołu", "lunch zespołu", "lunch zespołu"),
          ("haircut", "Fryzjer", "fryzjer", "fryzjera"), ("exam", "Egzamin", "egzamin", "egzamin"),
          ("parents' visit", "Wizyta rodziców", "wizyta rodziców", "wizytę rodziców")]
for (en_t, pl_t, pl_nom, pl_acc), (en_d, pl_d, pl_on, date) in zip(R.sample(EVENTS, len(EVENTS)), R.choices(DAYS, k=len(EVENTS))):
    h = R.choice([8, 9, 10, 11, 13, 14, 15, 16, 17, 18])
    ex(R.choice(["add {t} on {d} at {h}", "put {t} in the calendar for {d} {h}:00", "schedule {t} {d} at {h}"]).format(t=en_t, d=en_d, h=h),
       T("create_event", f"Added “{en_t.capitalize()}” on {en_d} {date} {h:02d}:00–{h + 1:02d}:00 to Personal.",
         title=en_t.capitalize(), start=f"{date} {h:02d}:00"),
       answer=f"Added {en_t} on {en_d} at {h:02d}:00.")
    ex(R.choice(["dodaj {a} {o} o {h}", "wpisz {a} {o} na {h}:00", "zapisz w kalendarzu {a} {o} o {h}"]).format(a=pl_acc, o=pl_on, h=h),
       T("create_event", f"Added “{pl_t}” on {en_d} {date} {h:02d}:00–{h + 1:02d}:00 to Personal.",
         title=pl_t, start=f"{date} {h:02d}:00"),
       answer=f"Dodałem: {pl_nom}, {pl_on} o {h}:00.")
for (en_d, pl_d, pl_on, date) in DAYS[:4]:
    items = R.sample(["09:00–10:00 Stand-up", "12:00–13:00 Lunch z Olą", "15:30–16:00 Dentysta", "18:00–19:30 Siłownia"], 2)
    res = f"{en_d} {date}:\n" + "\n".join(sorted(items))
    ex(R.choice(["what do I have on {d}", "am I free on {d}", "plans for {d}?"]).format(d=en_d),
       T("get_events", res, when=en_d.lower()),
       answer=f"On {en_d}: " + " and ".join(f"{i.split(' ', 1)[1]} at {i.split('–')[0]}" for i in sorted(items)) + ".")
    ex(R.choice(["co mam {o}?", "czy {o} mam coś w kalendarzu?"]).format(o=pl_on),
       T("get_events", res, when=en_d.lower()),
       answer=f"{pl_on.capitalize()}: " + ", ".join(f"{i.split(' ', 1)[1]} o {i.split('–')[0]}" for i in sorted(items)) + ".")

# ------------------------------------------------------------ mail
MAILS = [("Allegro", "Twoja paczka jest w drodze", "Przesyłka dotrze jutro między 10 a 14.", "the parcel arrives tomorrow between 10 and 14", "paczka dotrze jutro między 10 a 14"),
         ("PKP Intercity", "Bilet: Toruń → Gdańsk", "Pociąg 15:12, wagon 7, miejsce 54.", "train at 15:12, car 7, seat 54", "pociąg o 15:12, wagon 7, miejsce 54"),
         ("Orange", "Faktura za listopad", "Do zapłaty 89,99 zł do 25.11.", "89.99 zł due by 25 November", "do zapłaty 89,99 zł do 25 listopada"),
         ("GitHub", "[panda-assistant] PR #4 merged", "Wiertless merged 12 commits into main.", "PR #4 was merged into main", "PR #4 został scalony z main"),
         ("Uber Eats", "Twoje zamówienie", "Zamówienie z Pizzerii Roma dostarczone 19:42.", "the Pizzeria Roma order was delivered at 19:42", "zamówienie z Pizzerii Roma dotarło o 19:42")]
for sender, subj, body, en_sum, pl_sum in MAILS:
    res = f"Mail matching “{sender}”, newest first:\n[home:{R.randint(300, 399)}] {sender} — {subj} (Monday 2026-11-16 {R.randint(7, 21):02d}:{R.randint(0, 59):02d}, the newest)\n  {body}\nread_mail with an id in brackets reads the whole message."
    ex(R.choice(["any mail from {s}?", "what did {s} send me", "check my email from {s}"]).format(s=sender),
       T("search_mail", res, query=sender), answer=f"Yes: {subj}: {en_sum}.")
    ex(R.choice(["czy przyszło coś od {s}?", "co pisze {s}?", "sprawdź maila od {s}"]).format(s=sender),
       T("search_mail", res, query=sender), answer=f"Tak: „{subj}”: {pl_sum}.")
ex("any new mail", T("search_mail", "Mail matching “”, newest first:\n[home:401] Allegro — Twoja paczka jest w drodze (Monday 2026-11-16 09:12, the newest)\n  Przesyłka dotrze jutro.\n[uni:95] Dziekanat — Zmiana sali (Monday 2026-11-16 07:50)\n  Zajęcia w sali 204.", query=""),
   answer="The newest is from Allegro: your parcel arrives tomorrow. Before that, the dean's office moved a class to room 204.")
ex("mail from nobody@nothing", T("search_mail", "No mail matches “nobody@nothing”.", query="nobody@nothing"),
   answer="Nothing from that address.")

# ------------------------------------------------------------ the web, for what changes
LOOKUPS = [("bitcoin price", "Results from DuckDuckGo for 'bitcoin price': 1. Bitcoin (BTC) $71,240 - CoinMarketCap - 24h +1.8%…", "About $71,240 (CoinMarketCap), up 1.8% today."),
           ("Fedora 45 release date", "Results from DuckDuckGo for 'Fedora 45 release date': 1. Fedora 45 released October 28, 2026 - fedoramagazine.org…", "Fedora 45 came out on 28 October 2026 (Fedora Magazine)."),
           ("who won the last F1 race", "Results from DuckDuckGo for 'last F1 race winner': 1. Brazilian GP: Norris wins ahead of Verstappen - formula1.com…", "Lando Norris won the Brazilian GP, ahead of Verstappen (formula1.com)."),
           ("pogoda na weekend w Tatrach", "Results from DuckDuckGo for 'pogoda Tatry weekend': 1. Prognoza IMGW: sobota -4°C, śnieg; niedziela -2°C, słonecznie…", "Według IMGW w sobotę śnieg i -4°C, w niedzielę słońce i -2°C."),
           ("kurs dolara", "Results from DuckDuckGo for 'kurs dolara NBP': 1. USD 3,98 zł - kurs średni NBP (2026-11-16)…", "Średni kurs NBP: 3,98 zł za dolara."),
           ("najnowsza wersja GIMP-a", "Results from DuckDuckGo for 'GIMP najnowsza wersja': 1. GIMP 3.2 released - gimp.org…", "GIMP 3.2 (gimp.org).")]
for q, res, a in LOOKUPS:
    ex(q, T("web_search", res, query=q), answer=a)
ex("read me the top result about Fedora 45",
   T("web_search", "Results from DuckDuckGo for 'Fedora 45': 1. Fedora 45 is here - fedoramagazine.org/announcing-fedora-45 …", query="Fedora 45"),
   T("read_web_page", "Fedora 45 is here\nhttps://fedoramagazine.org/announcing-fedora-45\n\nFedora 45 brings GNOME 50, Linux 7.2, and the new Anaconda web installer by default…", url="https://fedoramagazine.org/announcing-fedora-45"),
   answer="Fedora 45 brings GNOME 50, Linux 7.2 and the new web installer by default (Fedora Magazine).")

# ------------------------------------------------------------ links and notes
SITES = [("youtube", "https://youtube.com"), ("allegro", "https://allegro.pl"), ("wikipedia", "https://wikipedia.org"),
         ("gmail", "https://mail.google.com"), ("the USOS page", "https://usosweb.uni.example.pl")]
for name, url in SITES:
    ex(R.choice(["open {n}", "go to {n}", "take me to {n}"]).format(n=name), T("open_url", f"Opened {url} in Firefox.", url=url))
for text_en, text_pl in [("the wifi password is kot123", "hasło do wifi to kot123"), ("buy milk and bread", "kup mleko i chleb"),
                         ("meeting moved to 3pm", "spotkanie przesunięte na 15")]:
    ex(f"send '{text_en}' to my phone", T("send_text", "Sent the note to jakubs-iphone with Taildrop.", device="jakubs-iphone", text=text_en),
       answer="Sent it to your iPhone.")
    ex(f"wyślij na telefon: {text_pl}", T("send_text", "Sent the note to jakubs-iphone with Taildrop.", device="jakubs-iphone", text=text_pl),
       answer="Wysłałem na iPhone'a.")

# ------------------------------------------------------------ media
for u, act, res, ans in [("pause the music", "pause", "Paused Gapless.", None), ("resume", "play", "Playing in Gapless.", None),
                         ("previous song", "previous_track", "Previous track in Gapless: Get Lucky.", None),
                         ("stop the music", "stop", "Stopped Gapless.", None),
                         ("następna piosenka", "next_track", "Next track in Gapless: Małomiasteczkowy.", "Następna: Małomiasteczkowy."),
                         ("wznów muzykę", "play", "Playing in Gapless.", "Gra dalej."),
                         ("co teraz gra?", "status", "Gapless is playing “Nie ma fal” by Dawid Podsiadło.", "„Nie ma fal”, Dawid Podsiadło.")]:
    ex(u, T("media_control", res, action=act), answer=ans)

# ------------------------------------------------------------ memory, clipboard
for fact_en, fact_pl in [("I work from home on Fridays", "w piątki pracuję z domu"), ("my favourite band is Daft Punk", "mój ulubiony zespół to Daft Punk"),
                         ("I'm allergic to nuts", "mam alergię na orzechy")]:
    ex(f"remember: {fact_en}", T("remember", f"Remembered: {fact_en[0].upper() + fact_en[1:]}.", fact=fact_en[0].upper() + fact_en[1:]))
    ex(f"zapamiętaj, że {fact_pl}", T("remember", f"Remembered: {fact_pl[0].upper() + fact_pl[1:]}.", fact=fact_pl[0].upper() + fact_pl[1:]),
       answer="Zapamiętałem.")
for q, res, a in [("what did I copy last", "Recent copies, newest first:\n1. ul. Długa 5, Toruń\n2. https://allegro.pl/oferta/123", "ul. Długa 5, Toruń"),
                  ("co ostatnio skopiowałem?", "Recent copies, newest first:\n1. 482913\n2. kot123", "Kod 482913.")]:
    ex(q, T("clipboard_history", res), answer=a)

# ------------------------------------------------------------ more everyday wordings
for u, tool, args, res, ans in [
    ("put the laptop in power saving", "set_power_mode", {"mode": "power-saver"}, "Power mode: power-saver.", None),
    ("balanced power please", "set_power_mode", {"mode": "balanced"}, "Power mode: balanced.", None),
    ("tryb zrównoważony", "set_power_mode", {"mode": "balanced"}, "Power mode: balanced.", "Włączyłem tryb zrównoważony."),
    ("show me the wifi networks", "list_networks", {}, "Networks: Home 5G (saved, connected, 92%), Biuro-Gosc (saved, 40%), FreeWifi (open, 35%)", "Connected to Home 5G; Biuro-Gosc and FreeWifi are also in range."),
    ("jakie mam urządzenia bluetooth?", "list_bluetooth", {}, "Paired: Jakub's AirPods Pro (connected), Sony WH-1000XM5, MX Master 3", "Sparowane: AirPods Pro (połączone), Sony WH-1000XM5 i MX Master 3."),
    ("what devices can I send to", "list_devices", {}, "Taildrop devices: jakubs-iphone (iOS, online), framework (linux, this computer)", "Your iPhone (online)."),
    ("how's the computer doing", "get_system_info", {}, "Framework Laptop 13, Ryzen AI 5 340, 16 GB (9.1 GB free), Fedora Silverblue 45, up 2 days", "All fine: 9 GB of 16 GB free, up for 2 days."),
    ("jaki mam system?", "get_system_info", {}, "Framework Laptop 13, Ryzen AI 5 340, 16 GB (9.1 GB free), Fedora Silverblue 45, up 2 days", "Fedora Silverblue 45 na Frameworku 13."),
    ("open the keyboard settings", "open_settings", {"panel": "keyboard"}, "Opened Settings → Keyboard.", None),
    ("otwórz ustawienia dźwięku", "open_settings", {"panel": "sound"}, "Opened Settings → Sound.", "Otworzyłem ustawienia dźwięku."),
]:
    ex(u, T(tool, res, **args), answer=ans)

# ------------------------------------------------------------ more small talk
for q, a in [("what can you do", "Change settings, find, read and send your files, edit pictures, check mail and the calendar, set reminders, look things up on the web, and more. Just ask."),
             ("co potrafisz?", "Zmieniam ustawienia, szukam i wysyłam pliki, edytuję zdjęcia, sprawdzam pocztę i kalendarz, ustawiam przypomnienia i szukam w internecie."),
             ("are you online?", "I run locally on your laptop; I only go online for web searches and the weather."),
             ("good night", "Good night!"), ("dobranoc", "Dobranoc!"),
             ("how do I say 'thank you' in German", "Danke (or danke schön)."),
             ("jak jest 'kot' po angielsku?", "Cat."),
             ("give me a motivational quote", "“Done is better than perfect.”")]:
    chat(q, a)

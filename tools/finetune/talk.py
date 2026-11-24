"""Hand-written talk vs. requests (Claude, 2026-11-24): the eval's no-tool family
passes 16/27; Gemma calls a tool when the user is only talking. Remarks that
sound like requests stay talk (answered through reply), next to the real
request that looks almost the same.
"""

from data_dsl import T, chat, ex

# remarks that only sound like requests
for q, a in [
    ("I love dark mode", "Same here, easier on the eyes at night. Want it on?"),
    ("my phone died yesterday", "Oh no! Was it the battery or something worse?"),
    ("so much email lately", "Sounds exhausting. Want me to show just the newest few?"),
    ("the weather has been awful this week", "Grey and wet, yes. Hopefully the weekend is better."),
    ("I always forget to water my plants", "Happens to everyone. I can set a weekly reminder if you'd like."),
    ("music helps me focus", "Same for many people: instrumental tracks work best for most."),
    ("I hate when the volume is too loud at startup", "That's annoying. I can turn it down whenever you ask."),
    ("I took a lot of photos on holiday", "Nice! Where did you go?"),
    ("my sister's birthday is next week", "Lovely. Want me to put it in the calendar or set a reminder?"),
    ("I have an exam on Friday", "Good luck! Should I add it to your calendar?"),
    ("uwielbiam ciemny motyw", "Ja też, oczy mniej się męczą. Włączyć?"),
    ("ale dziś zimno", "Prawdziwa jesień. Sprawdzić prognozę?"),
    ("zawsze zapominam o rachunkach", "Mogę ustawić przypomnienie na termin płatności, jeśli chcesz."),
    ("mam dużo zdjęć z wakacji", "Super! Gdzie byliście?"),
    ("jutro mam egzamin", "Powodzenia! Dodać go do kalendarza?"),
    ("nie lubię poniedziałków", "Mało kto lubi. Kawa pomaga!"),
    ("mój laptop jest super", "Framework to dobry wybór!"),
    ("czasem gram w Steam wieczorami", "Fajnie! W co ostatnio grasz?"),
]:
    chat(q, a)

# the real request that looks almost the same
ex("put the dark mode on, I love it", T("set_dark_mode", "Dark mode on.", enabled=True))
ex("my sister's birthday is next friday, put it in the calendar",
   T("create_event", "Added “Sister's birthday” on Friday 2026-11-20 (all day) to Personal.", title="Sister's birthday", start="2026-11-20"),
   answer="Added your sister's birthday on Friday 20 November.")
ex("I have an exam on friday at 10, add it",
   T("create_event", "Added “Exam” on Friday 2026-11-20 10:00–11:00 to Personal.", title="Exam", start="2026-11-20 10:00"),
   answer="Added the exam on Friday at 10:00.")
ex("remind me every sunday to water the plants",
   T("set_reminder", "I'll remind you on Sunday at 10:00: water the plants. (One reminder; ask again for the next week.)", at="2026-11-22 10:00", text="water the plants"),
   answer="Set for Sunday at 10:00. Reminders here are one-off, so ask again next week.")
ex("jutro mam egzamin o 9, wpisz go",
   T("create_event", "Added “Egzamin” on Monday 2026-11-16 09:00–10:00 to Personal.", title="Egzamin", start="2026-11-16 09:00"),
   answer="Dodałem egzamin jutro o 9:00.")
ex("zimno, jaka jest prognoza na jutro w Toruniu?",
   T("get_weather", "Toruń now: -1°C, clear.\nTomorrow: -3–2°C, sunny.", place="Toruń"),
   answer="Jutro w Toruniu słonecznie, od -3 do 2°C.")
ex("yes, turn it on",
   T("set_dark_mode", "Dark mode on.", enabled=True),
   before=[("I love dark mode", "Same here, easier on the eyes at night. Want it on?")])
ex("tak, wpisz",
   T("create_event", "Added “Egzamin” on Monday 2026-11-16 (all day) to Personal.", title="Egzamin", start="2026-11-16"),
   answer="Dodałem egzamin na jutro.",
   before=[("jutro mam egzamin", "Powodzenia! Dodać go do kalendarza?")])
chat("no thanks", "Okay!", before=[("I have an exam on Friday", "Good luck! Should I add it to your calendar?")])
chat("nie, dzięki", "Jasne!", before=[("uwielbiam ciemny motyw", "Ja też, oczy mniej się męczą. Włączyć?")])

# questions about how, not requests to do
for q, a in [
    ("how do I turn on dark mode myself", "Settings → Appearance → Dark, or the Dark Style button in the quick settings menu at the top right."),
    ("how do I take a screenshot", "Press Print Screen: GNOME's screenshot tool opens; or Shift+Print for the whole screen at once."),
    ("jak samemu wyciszyć powiadomienia?", "Kliknij zegar na górze i włącz „Nie przeszkadzać” pod listą powiadomień."),
    ("what's the shortcut to switch windows", "Super+Tab between apps, Alt+` between windows of the same app."),
    ("jak zainstalować aplikację na Silverblue?", "Najprościej z Centrum oprogramowania (Flatpak); narzędzia dla programistów w toolboxie."),
]:
    chat(q, a)

"""Balancing batch (Claude, 2026-11-24), from ft1's regressions on the NPU eval: the
first adapter learned "a document -> ask_document" and "to my phone -> send_file"
too well (32 and 60 examples, against 2 for read_file and 14 for send_text), and
looped on find_files when nothing matched (2 examples of an empty result). Here:
short text files are read with read_file; text the user gives goes to the phone
with send_text; "make a file that says…" is one write_file; what's open is read
with read_window and answered from it; the light theme is set_dark_mode off;
"the newest version of X" is a web search; a folder the user names goes straight
to the tool; an empty find_files gets one other try at most, then the answer says
so; answers name the fact that was asked for.
"""

from data_dsl import T, ex

PHONE = "jakubs-iphone"

# ------------------------------------------------------------ short text files: read_file
for u, path, text, ans in [
    ("what did I put in notes.txt", "~/Dokumenty/notes.txt", "Call the landlord about the heater.\nRenew the bus pass.\nBuy a birthday card for Ola.",
     "Three things: call the landlord about the heater, renew the bus pass, buy a birthday card for Ola."),
    ("read me shopping.md from the desktop", "~/Pulpit/shopping.md", "- oat milk\n- rye bread\n- apples\n- coffee beans",
     "Oat milk, rye bread, apples and coffee beans."),
    ("co mam w pliku zadania.txt?", "~/Dokumenty/zadania.txt", "1. Oddać książki do biblioteki\n2. Zapłacić za prąd\n3. Umówić przegląd auta",
     "Trzy zadania: oddać książki do biblioteki, zapłacić za prąd i umówić przegląd auta."),
    ("przeczytaj mi plik lista.md z pulpitu", "~/Pulpit/lista.md", "- chleb\n- masło\n- pomidory",
     "Chleb, masło i pomidory."),
    ("what's in my packing list", "~/Dokumenty/packing.txt", "passport\ncharger\nswimsuit\nsunscreen",
     "Passport, charger, swimsuit and sunscreen."),
    ("show me what my ideas.md says", "~/Dokumenty/ideas.md", "# Ideas\n- a widget for the bus timetable\n- a recipe scaler",
     "Two ideas: a bus timetable widget and a recipe scaler."),
    ("what port is set in ~/.config/app/config.toml", "~/.config/app/config.toml", "[server]\nhost = \"127.0.0.1\"\nport = 8080",
     "Port 8080, on 127.0.0.1."),
]:
    ex(u, T("read_file", text, path=path), answer=ans)
# a long document with a question about it stays ask_document (the contrast)
ex("what does my lease say about subletting",
   T("find_files", "~/Dokumenty/umowa najmu 2026.pdf", name="umowa najmu"),
   T("ask_document", "From ~/Dokumenty/umowa najmu 2026.pdf: § 12. Podnajem lokalu wymaga pisemnej zgody Wynajmującego.",
     path="~/Dokumenty/umowa najmu 2026.pdf", question="what does it say about subletting"),
   answer="Subletting needs the landlord's written consent (§ 12).")

# ------------------------------------------------------------ text the user gives -> send_text
for u, text, ans in [
    ("text my phone the gate code 4471", "Gate code: 4471", "Sent the gate code to your iPhone."),
    ("put this on my phone: pick up the parcel at the Żabka on Długa", "Pick up the parcel at the Żabka on Długa", "Sent it to your iPhone."),
    ("send a note to my iphone saying call the bank tomorrow", "Call the bank tomorrow", "Sent the note to your iPhone."),
    ("wyślij na telefon listę: pomidory, ser, makaron", "Pomidory, ser, makaron", "Wysłałem listę na iPhone'a."),
    ("prześlij mi na telefon adres: ul. Szeroka 12, Toruń", "ul. Szeroka 12, Toruń", "Wysłałem adres na iPhone'a."),
    ("send the recipe steps to my phone: boil pasta 9 min, fry garlic, mix with parmesan", "Boil pasta 9 min, fry garlic, mix with parmesan", "Sent the recipe to your iPhone."),
]:
    ex(u, T("send_text", f"Sent the note to {PHONE} with Taildrop.", device=PHONE, text=text), answer=ans)
ex("put that on my phone please",
   T("send_text", f"Sent the note to {PHONE} with Taildrop.", device=PHONE, text="Bus 27 leaves at 7:42 from Plac Rapackiego."),
   answer="Sent it to your iPhone.",
   before=[("when does my bus leave", "Bus 27 leaves at 7:42 from Plac Rapackiego.")])
ex("wyślij to na telefon",
   T("send_text", f"Sent the note to {PHONE} with Taildrop.", device=PHONE, text="Apteka przy Chełmińskiej jest otwarta do 22:00."),
   answer="Wysłałem na iPhone'a.",
   before=[("do której jest otwarta apteka na Chełmińskiej?", "Apteka przy Chełmińskiej jest otwarta do 22:00.")])

# ------------------------------------------------------------ "make a file that says…" -> one write_file
for u, path, content, ans in [
    ("create groceries.txt on the desktop with eggs, flour and butter", "~/Pulpit/groceries.txt", "eggs\nflour\nbutter", "Made groceries.txt on the desktop."),
    ("make a file called reminder.txt in documents saying renew passport in March", "~/Dokumenty/reminder.txt", "Renew passport in March", "Made reminder.txt in Documents."),
    ("utwórz plik zakupy.txt na pulpicie: mleko, jajka, chleb", "~/Pulpit/zakupy.txt", "mleko\njajka\nchleb", "Utworzyłem zakupy.txt na pulpicie."),
    ("zapisz w pliku notatka.txt w dokumentach, że hasło do routera jest na naklejce", "~/Dokumenty/notatka.txt", "Hasło do routera jest na naklejce.", "Zapisałem notatka.txt w Dokumentach."),
]:
    ex(u, T("write_file", f"Wrote {path}.", path=path, content=content), answer=ans)

# ------------------------------------------------------------ what's open: read_window, answer from it
for u, window, ans in [
    ("give me the gist of this page", "Firefox — “Framework Laptop 16 review”\nThe Framework Laptop 16 is repairable and modular; battery life is about 6 hours; the GPU module adds weight…",
     "A Framework Laptop 16 review: repairable and modular, about 6 hours of battery, and the GPU module adds weight."),
    ("tl;dr of what I'm reading", "Firefox — “Why Rust in the kernel”\nRust drivers avoid memory bugs that make up about two thirds of kernel CVEs; the first drivers landed in 6.8…",
     "Rust in the kernel: Rust drivers avoid the memory bugs behind about two thirds of kernel CVEs; the first ones landed in 6.8."),
    ("streść ten artykuł", "Firefox — “Tramwaje w Toruniu: nowa linia”\nOd marca linia 6 pojedzie przez Jar; tramwaje co 10 minut w szczycie…",
     "Od marca linia 6 pojedzie przez Jar, w szczycie co 10 minut."),
    ("o czym jest ten mail?", "Convey — “Zebranie wspólnoty”\nZebranie wspólnoty mieszkaniowej w czwartek o 18:00 w świetlicy; głosowanie nad remontem dachu.",
     "Zebranie wspólnoty w czwartek o 18:00 w świetlicy, z głosowaniem nad remontem dachu."),
]:
    ex(u, T("read_window", window), answer=ans)

# ------------------------------------------------------------ the light theme is dark mode off
for u, ans in [("switch to the light theme", "Light theme on."), ("turn dark mode off", "Dark mode off."),
               ("light mode please", "Light theme on."), ("wróć do jasnego motywu", "Jasny motyw włączony."),
               ("jasny motyw", "Jasny motyw włączony."), ("wyłącz już ten ciemny motyw", "Ciemny motyw wyłączony.")]:
    ex(u, T("set_dark_mode", "Dark mode off.", enabled=False), answer=ans)

# ------------------------------------------------------------ the newest version of something: the web
for u, q, res, ans in [
    ("what's the latest version of GNOME", "latest GNOME release",
     "Results from DuckDuckGo for 'latest GNOME release': 1. GNOME 51 released - release.gnome.org/51 …", "GNOME 51 (release.gnome.org)."),
    ("newest Linux kernel?", "latest Linux kernel version",
     "Results from DuckDuckGo for 'latest Linux kernel version': 1. kernel.org: stable 7.4.2, mainline 7.5-rc3 …", "Stable 7.4.2; 7.5 is at rc3 (kernel.org)."),
    ("jaka jest najnowsza wersja Firefoksa?", "najnowsza wersja Firefox",
     "Results from DuckDuckGo for 'najnowsza wersja Firefox': 1. Firefox 152.0 release notes - mozilla.org …", "Firefox 152.0 (mozilla.org)."),
]:
    ex(u, T("web_search", res, query=q), answer=ans)
ex("which Fedora am I on", T("get_system_info", "Fedora Linux 45 (Silverblue), kernel 7.3.4, up 2 days, 32 GB RAM (11 GB used), disk 268 of 475 GB used."),
   answer="Fedora 45 Silverblue.")

# ------------------------------------------------------------ a folder the user names goes straight to the tool
ex("shrink the photos in Pictures/Hike and send them to my phone",
   T("edit_image", "Saved 12 of 12 in ~/Obrazy/Edited (the originals are unchanged):\n~/Obrazy/Edited/IMG_2201.jpg: 3.8 MB 4032×3024 → 0.5 MB 1920×1440\n…", files="Pictures/Hike", resize="1920"),
   T("send_file", f"Sent 12 files from ~/Obrazy/Edited to {PHONE} with Taildrop.", device=PHONE, path="~/Obrazy/Edited"),
   answer="Shrank 12 photos (about 0.5 MB each) and sent them to your iPhone; the originals are unchanged.")
ex("zmniejsz zdjęcia z Obrazy/Wesele i wyślij je na telefon",
   T("edit_image", "Saved 30 of 30 in ~/Obrazy/Edited (the originals are unchanged):\n~/Obrazy/Edited/DSC0001.jpg: 6.1 MB 6000×4000 → 0.7 MB 1920×1280\n…", files="Pictures/Wesele", resize="1920"),
   T("send_file", f"Sent 30 files from ~/Obrazy/Edited to {PHONE} with Taildrop.", device=PHONE, path="~/Obrazy/Edited"),
   answer="Zmniejszyłem 30 zdjęć i wysłałem je na iPhone'a; oryginały zostały bez zmian.")

# ------------------------------------------------------------ nothing found: one other try, then say so
ex("open my insurance letter",
   T("find_files", "No files named like 'insurance letter' in ~.", name="insurance letter"),
   T("find_files", "No files named like 'ubezpieczenie' in ~.", name="ubezpieczenie"),
   answer="I couldn't find an insurance letter in your files (I tried “insurance letter” and “ubezpieczenie”). What's the file called?")
ex("znajdź skan dowodu",
   T("find_files", "No files named like 'skan dowodu' in ~.", name="skan dowodu"),
   T("find_files", "No files named like 'dowód' in ~.", name="dowód"),
   answer="Nie znalazłem skanu dowodu (szukałem „skan dowodu” i „dowód”). Jak nazywa się ten plik?")
ex("where's my tax return",
   T("find_files", "No files named like 'tax return' in ~.", name="tax return"),
   T("find_files", "~/Dokumenty/PIT-37 2025.pdf", name="PIT"),
   answer="~/Dokumenty/PIT-37 2025.pdf")
ex("what port is the music server on",
   T("search_text", "No matches for 'music server' in ~/Dokumenty.", query="music server", folder="Documents"),
   answer="I couldn't find anything about a music server in your documents. Which app is it?")

# ------------------------------------------------------------ answers name what was asked
ex("find my passport scan",
   T("find_files", "~/Dokumenty/skany/paszport.pdf", name="paszport"),
   answer="~/Dokumenty/skany/paszport.pdf")
ex("gdzie jest moja umowa o pracę?",
   T("find_files", "~/Dokumenty/praca/umowa o pracę 2025.pdf", name="umowa o pracę"),
   answer="W Dokumenty/praca: umowa o pracę 2025.pdf.")
ex("play Levitating and tell me who sings it",
   T("play_music", "Playing Levitating by Dua Lipa (~/Muzyka/Dua Lipa/Levitating.mp3) in Gapless.", query="Levitating"),
   answer="Levitating by Dua Lipa is playing.")
ex("find songs by Sanah and send one to my phone",
   T("find_files", "~/Muzyka/Sanah/Szampan.flac\n~/Muzyka/Sanah/Ale jazz!.flac", name="Sanah", folder="Music"),
   T("send_file", f"Sent Szampan.flac to {PHONE} with Taildrop.", device=PHONE, path="~/Muzyka/Sanah/Szampan.flac"),
   answer="Sent “Szampan” by Sanah to your iPhone.")

PREFIX ?= /usr/local
MANDIR := $(PREFIX)/share/man

install:
	install -d "$(DESTDIR)$(MANDIR)/man1"
	install -m 0644 man/readertui.1 "$(DESTDIR)$(MANDIR)/man1/readertui.1"

uninstall:
	rm -f "$(DESTDIR)$(MANDIR)/man1/readertui.1"

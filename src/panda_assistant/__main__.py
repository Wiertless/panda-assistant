import sys


def main() -> int:
    # The search provider must not load GTK: it runs headless on D-Bus.
    if "--search-provider" in sys.argv[1:]:
        from .search_provider import main as provider_main
        return provider_main()
    from .app import main as app_main
    return app_main(sys.argv)


if __name__ == "__main__":
    sys.exit(main())

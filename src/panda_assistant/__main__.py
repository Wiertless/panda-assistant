import sys


def main() -> int:
    from .app import main as app_main
    return app_main(sys.argv)


if __name__ == "__main__":
    sys.exit(main())

"""Download resumable Binance Spot klines into the CSV cache."""

import argparse

import barq


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--symbols", default="BTCUSDT,ETHUSDT")
    parser.add_argument("--interval", default="1d")
    parser.add_argument("--from", dest="start", default="2021-01-01")
    parser.add_argument("--dir", default="data/binance")
    args = parser.parse_args()

    symbols = [symbol.strip() for symbol in args.symbols.split(",") if symbol.strip()]
    failed = 0
    for symbol in symbols:
        try:
            bars = barq.data.fetch_cached(
                symbol, args.interval, args.start, directory=args.dir
            )
            span = f"{bars[0].open_time} .. {bars[-1].open_time}" if bars else "empty"
            print(f"{symbol:<12} {len(bars):>7} bars  {span}")
        except Exception as error:
            print(f"{symbol:<12} failed: {error}")
            failed += 1
    if failed:
        raise SystemExit(f"{failed} of {len(symbols)} symbols failed")


if __name__ == "__main__":
    main()

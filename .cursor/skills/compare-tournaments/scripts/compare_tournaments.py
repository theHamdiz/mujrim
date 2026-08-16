#!/usr/bin/env python3
"""Compare two Mujrim GUI tournaments from study SQLite + duel checkpoints."""

from __future__ import annotations

import argparse
import json
import sqlite3
import sys
import unittest
from collections import defaultdict
from pathlib import Path

DISPLAY = {
    "akimbo": "Akimbo",
    "ethereal": "Ethereal",
    "hobbes": "Hobbes",
    "integral": "Integral",
    "lc0": "Lc0",
    "mujrim-ak": "Mujrim Akimbo",
    "mujrim-akimbo": "Mujrim Akimbo",
    "mujrim-ateed": "Mujrim Ateed",
    "mujrim-elite": "Mujrim Elite",
    "mujrim-external": "Mujrim External",
    "mujrim-lc0": "Mujrim Lc0",
    "mujrim-obs": "Mujrim Obsidian",
    "mujrim-obsidian": "Mujrim Obsidian",
    "mujrim-plenty": "Mujrim PlentyChess",
    "mujrim-plentychess": "Mujrim PlentyChess",
    "mujrim-v10": "Mujrim Elite",
    "mujrim-v60": "Mujrim v60",
    "mujrim-viri": "Mujrim Viridithas",
    "mujrim-viridithas": "Mujrim Viridithas",
    "obsidian": "Obsidian",
    "plentychess": "PlentyChess",
    "reckless": "Reckless",
    "stockfish": "Stockfish",
    "velvet": "Velvet",
    "viridithas": "Viridithas",
}

# Product binary → upstream native. Elite is Stockfish; v60 is Reckless.
ADAPTER_NATIVE = {
    "Mujrim Akimbo": "Akimbo",
    "Mujrim Elite": "Stockfish",
    "Mujrim Lc0": "Lc0",
    "Mujrim Obsidian": "Obsidian",
    "Mujrim PlentyChess": "PlentyChess",
    "Mujrim v60": "Reckless",
    "Mujrim Viridithas": "Viridithas",
}

FORMAT_DIR = {
    "round_robin": "round-robin",
    "double_round_robin": "double-round-robin",
    "swiss": "swiss",
    "knockout": "knockout",
}


def default_roots() -> tuple[Path, Path]:
    data = Path.home() / ".local" / "share" / "Mujrim"
    return data / "library" / "mujrim.sqlite3", data / "tournaments"


def display_name(stem_or_label: str) -> str:
    key = Path(stem_or_label).stem.lower()
    return DISPLAY.get(key, stem_or_label)


def outcome_score(outcome: str) -> float:
    if outcome == "win":
        return 1.0
    if outcome == "loss":
        return 0.0
    return 0.5


def parse_checkpoint(path: Path) -> list[dict]:
    lines = [line for line in path.read_text().splitlines() if line.strip()]
    if not lines:
        return []
    header = json.loads(lines[0])
    if header.get("type") != "mujrim-duel-checkpoint":
        return []
    cand = display_name(header["candidate"]["path"])
    ref = display_name(header["reference"]["path"])
    games = []
    for line in lines[1:]:
        rec = json.loads(line)
        if rec.get("type") != "pair":
            continue
        for key, white, black, cand_white in (
            ("candidate_white", cand, ref, True),
            ("candidate_black", ref, cand, False),
        ):
            game = rec[key]
            cand_pts = outcome_score(game["outcome"])
            white_score = cand_pts if cand_white else 1.0 - cand_pts
            tel = game.get("telemetry") or {}
            games.append(
                {
                    "white": white,
                    "black": black,
                    "white_score": white_score,
                    "termination": game.get("termination"),
                    "detail": game.get("detail"),
                    "white_tel": tel.get("candidate" if cand_white else "reference")
                    or {},
                    "black_tel": tel.get("reference" if cand_white else "candidate")
                    or {},
                    "cand_mtime": header["candidate"].get("modified_ms"),
                    "ref_mtime": header["reference"].get("modified_ms"),
                    "hash_mb": header.get("hash_mb"),
                    "engine_threads": header.get("engine_threads"),
                }
            )
    return games


def load_checkpoint_games(event_dir: Path) -> list[dict]:
    games: list[dict] = []
    if not event_dir.is_dir():
        return games
    for path in sorted(event_dir.glob("*.jsonl")):
        games.extend(parse_checkpoint(path))
    return games


def load_sqlite_games(db: Path, tournament_id: str) -> list[dict]:
    if not db.is_file():
        return []
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = con.execute(
        "SELECT white, black, white_score FROM tournament_games WHERE tournament_id=?",
        (tournament_id,),
    ).fetchall()
    con.close()
    return [
        {
            "white": white,
            "black": black,
            "white_score": float(score),
            "termination": None,
            "detail": None,
            "white_tel": {},
            "black_tel": {},
        }
        for white, black, score in rows
    ]


def list_tournaments(db: Path) -> list[dict]:
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = con.execute(
        """
        SELECT id, name, format, status, created_at,
               (SELECT COUNT(*) FROM tournament_games g WHERE g.tournament_id = t.id)
        FROM tournaments t
        ORDER BY created_at DESC
        """
    ).fetchall()
    con.close()
    return [
        {
            "id": row[0],
            "name": row[1],
            "format": row[2],
            "status": row[3],
            "created_at": row[4],
            "sqlite_games": row[5],
        }
        for row in rows
    ]


def resolve_event(events: list[dict], needle: str) -> dict:
    if needle.startswith("t-") or needle.startswith("t_"):
        for event in events:
            if event["id"] == needle:
                return event
    lowered = needle.casefold()
    matches = [event for event in events if event["name"].casefold() == lowered]
    if len(matches) == 1:
        return matches[0]
    matches = [event for event in events if lowered in event["name"].casefold()]
    if len(matches) == 1:
        return matches[0]
    if not matches:
        known = ", ".join(f"{e['name']} ({e['id']})" for e in events[:8])
        raise SystemExit(f"No tournament matches {needle!r}. Known: {known}")
    ids = ", ".join(f"{e['name']} ({e['id']})" for e in matches)
    raise SystemExit(f"Ambiguous tournament {needle!r}: {ids}")


def event_dir(tournaments_root: Path, event: dict) -> Path:
    folder = FORMAT_DIR.get(event["format"], event["format"].replace("_", "-"))
    return tournaments_root / folder / event["id"]


def prefer_games(checkpoint: list[dict], sqlite: list[dict]) -> list[dict]:
    if len(checkpoint) >= len(sqlite) and checkpoint:
        return checkpoint
    if sqlite:
        return sqlite
    return checkpoint


def standings(games: list[dict], field: set[str] | None = None) -> list[dict]:
    stats = defaultdict(
        lambda: {
            "played": 0,
            "wins": 0,
            "draws": 0,
            "losses": 0,
            "points": 0.0,
            "forfeits": 0,
            "nps": [],
            "depth": [],
        }
    )
    for game in games:
        if field and (game["white"] not in field or game["black"] not in field):
            continue
        pair = (
            (game["white"], game["white_score"], game.get("white_tel") or {}),
            (game["black"], 1.0 - game["white_score"], game.get("black_tel") or {}),
        )
        for name, pts, tel in pair:
            row = stats[name]
            row["played"] += 1
            row["points"] += pts
            if pts == 1.0:
                row["wins"] += 1
            elif pts == 0.5:
                row["draws"] += 1
            else:
                row["losses"] += 1
            if game.get("termination") == "forfeit" and pts == 0.0:
                row["forfeits"] += 1
            if tel.get("nps"):
                row["nps"].append(float(tel["nps"]))
            if tel.get("average_depth"):
                row["depth"].append(float(tel["average_depth"]))
    out = []
    for name, row in stats.items():
        played = row["played"]
        out.append(
            {
                "name": name,
                "played": played,
                "wins": row["wins"],
                "draws": row["draws"],
                "losses": row["losses"],
                "points": row["points"],
                "score_pct": (100.0 * row["points"] / played) if played else 0.0,
                "forfeits": row["forfeits"],
                "avg_nps": (sum(row["nps"]) / len(row["nps"])) if row["nps"] else 0.0,
                "avg_depth": (sum(row["depth"]) / len(row["depth"]))
                if row["depth"]
                else 0.0,
            }
        )
    out.sort(key=lambda row: (-row["points"], -row["wins"], row["name"]))
    for index, row in enumerate(out, 1):
        row["rank"] = index
    return out


def h2h(games: list[dict], left: str, right: str) -> dict:
    wins = draws = losses = 0
    for game in games:
        if {game["white"], game["black"]} != {left, right}:
            continue
        pts = game["white_score"] if game["white"] == left else 1.0 - game["white_score"]
        if pts == 1.0:
            wins += 1
        elif pts == 0.5:
            draws += 1
        else:
            losses += 1
    played = wins + draws + losses
    return {
        "left": left,
        "right": right,
        "played": played,
        "wins": wins,
        "draws": draws,
        "losses": losses,
        "points": wins + 0.5 * draws,
        "score_pct": (100.0 * (wins + 0.5 * draws) / played) if played else 0.0,
    }


def vs_set(games: list[dict], name: str, opponents: set[str]) -> dict:
    subset = [
        game
        for game in games
        if name in (game["white"], game["black"])
        and (
            game["black"] if game["white"] == name else game["white"]
        )
        in opponents
    ]
    wins = draws = losses = 0
    for game in subset:
        pts = game["white_score"] if game["white"] == name else 1.0 - game["white_score"]
        if pts == 1.0:
            wins += 1
        elif pts == 0.5:
            draws += 1
        else:
            losses += 1
    played = wins + draws + losses
    return {
        "played": played,
        "wins": wins,
        "draws": draws,
        "losses": losses,
        "points": wins + 0.5 * draws,
        "score_pct": (100.0 * (wins + 0.5 * draws) / played) if played else 0.0,
    }


def finished_only(games: list[dict]) -> list[dict]:
    if not any(game.get("termination") for game in games):
        return games
    return [game for game in games if game.get("termination") != "forfeit"]


def print_table(title: str, rows: list[dict]) -> None:
    print(f"\n=== {title} ===")
    print(
        f"{'Rk':>3}  {'Engine':<22} {'P':>4} {'W':>3} {'D':>3} {'L':>3} "
        f"{'Pts':>6} {'%':>6} {'NPS':>9} {'Depth':>6} {'Flag':>4}"
    )
    for row in rows:
        print(
            f"{row['rank']:3d}  {row['name']:<22} {row['played']:4d} {row['wins']:3d} "
            f"{row['draws']:3d} {row['losses']:3d} {row['points']:6.1f} "
            f"{row['score_pct']:5.1f}% {row['avg_nps']:9.0f} {row['avg_depth']:6.1f} "
            f"{row['forfeits']:4d}"
        )


def compare(before: dict, after: dict) -> dict:
    field = {row["name"] for row in after["standings"]}
    before_sub = standings(before["games"], field)
    after_stand = after["standings"]
    before_map = {row["name"]: row for row in before_sub}
    after_map = {row["name"]: row for row in after_stand}
    deltas = []
    for name in sorted(field, key=lambda item: after_map[item]["rank"]):
        old = before_map.get(name)
        new = after_map[name]
        deltas.append(
            {
                "name": name,
                "before_rank": old["rank"] if old else None,
                "after_rank": new["rank"],
                "before_pct": old["score_pct"] if old else None,
                "after_pct": new["score_pct"],
                "delta_pp": (new["score_pct"] - old["score_pct"]) if old else None,
                "before_nps": old["avg_nps"] if old else 0.0,
                "after_nps": new["avg_nps"],
                "adapter": name in ADAPTER_NATIVE,
                "native": ADAPTER_NATIVE.get(name),
            }
        )
    natives_present = {native for native in ADAPTER_NATIVE.values() if native in field}
    adapter_rows = []
    for adapter, native in ADAPTER_NATIVE.items():
        if adapter not in field:
            continue
        adapter_rows.append(
            {
                "adapter": adapter,
                "native": native,
                "h2h_before": h2h(before["games"], adapter, native),
                "h2h_after": h2h(after["games"], adapter, native),
                "vs_natives_before": vs_set(before["games"], adapter, natives_present),
                "vs_natives_after": vs_set(after["games"], adapter, natives_present),
            }
        )
    return {
        "before_subset": before_sub,
        "deltas": deltas,
        "adapters": adapter_rows,
        "finished_after": standings(finished_only(after["games"])),
    }


def load_event(db: Path, tournaments_root: Path, event: dict) -> dict:
    disk = load_checkpoint_games(event_dir(tournaments_root, event))
    sqlite = load_sqlite_games(db, event["id"])
    games = prefer_games(disk, sqlite)
    return {
        **event,
        "games": games,
        "checkpoint_games": len(disk),
        "source": "checkpoints" if games is disk or len(disk) >= len(sqlite) else "sqlite",
        "standings": standings(games),
        "hash_mb": next((game.get("hash_mb") for game in disk if game.get("hash_mb")), None),
        "engine_threads": next(
            (game.get("engine_threads") for game in disk if game.get("engine_threads")),
            None,
        ),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "after",
        nargs="?",
        help="Newer event name or id (default: latest finished)",
    )
    parser.add_argument(
        "before",
        nargs="?",
        help="Baseline event name or id (default: previous finished)",
    )
    parser.add_argument("--db", type=Path, default=None)
    parser.add_argument("--tournaments", type=Path, default=None)
    parser.add_argument("--json", type=Path, default=None)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(CompareTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1

    db, tournaments_root = default_roots()
    if args.db:
        db = args.db
    if args.tournaments:
        tournaments_root = args.tournaments
    events = list_tournaments(db)
    finished = [event for event in events if event["status"] == "finished"]
    if args.after:
        after_meta = resolve_event(events, args.after)
    elif finished:
        after_meta = finished[0]
    else:
        raise SystemExit("No finished tournaments in the study database.")
    if args.before:
        before_meta = resolve_event(events, args.before)
    else:
        rest = [event for event in finished if event["id"] != after_meta["id"]]
        if not rest:
            raise SystemExit("Need a second finished tournament for a baseline.")
        before_meta = rest[0]

    after = load_event(db, tournaments_root, after_meta)
    before = load_event(db, tournaments_root, before_meta)
    report = compare(before, after)
    payload = {
        "before": {
            "id": before["id"],
            "name": before["name"],
            "games": len(before["games"]),
            "source": before["source"],
            "hash_mb": before["hash_mb"],
            "standings": before["standings"],
        },
        "after": {
            "id": after["id"],
            "name": after["name"],
            "games": len(after["games"]),
            "source": after["source"],
            "hash_mb": after["hash_mb"],
            "standings": after["standings"],
        },
        **report,
    }
    print(
        f"{after['name']} ({after['id']}, {len(after['games'])} games, {after['source']})"
        f" vs {before['name']} ({before['id']}, {len(before['games'])} games, {before['source']})"
    )
    print_table(f"{after['name']} standings", after["standings"])
    print_table(f"{before['name']} restricted to later field", report["before_subset"])
    print_table(f"{after['name']} excluding time forfeits", report["finished_after"])
    print("\n=== Adapter vs native ===")
    for row in report["adapters"]:
        old = row["h2h_before"]
        new = row["h2h_after"]
        print(
            f"{row['adapter']} vs {row['native']}: "
            f"before {old['wins']}-{old['draws']}-{old['losses']} ({old['score_pct']:.1f}%) → "
            f"after {new['wins']}-{new['draws']}-{new['losses']} ({new['score_pct']:.1f}%)"
        )
    if args.json:
        args.json.write_text(json.dumps(payload, indent=2))
        print(f"\nWrote {args.json}")
    else:
        print("\n--- json ---")
        print(json.dumps(payload, indent=2))
    return 0


class CompareTests(unittest.TestCase):
    def test_display_maps_product_stems(self) -> None:
        self.assertEqual(display_name("mujrim-viri"), "Mujrim Viridithas")
        self.assertEqual(display_name("mujrim-v60"), "Mujrim v60")
        self.assertEqual(ADAPTER_NATIVE["Mujrim Elite"], "Stockfish")
        self.assertEqual(ADAPTER_NATIVE["Mujrim v60"], "Reckless")

    def test_standings_and_h2h(self) -> None:
        games = [
            {
                "white": "Mujrim Obsidian",
                "black": "Obsidian",
                "white_score": 1.0,
                "termination": "adjudicated_win",
                "white_tel": {"nps": 500_000, "average_depth": 12.0},
                "black_tel": {"nps": 2_000_000, "average_depth": 28.0},
            },
            {
                "white": "Obsidian",
                "black": "Mujrim Obsidian",
                "white_score": 0.5,
                "termination": "adjudicated_draw",
                "white_tel": {"nps": 2_000_000, "average_depth": 28.0},
                "black_tel": {"nps": 500_000, "average_depth": 12.0},
            },
            {
                "white": "Ethereal",
                "black": "Mujrim Obsidian",
                "white_score": 0.0,
                "termination": "forfeit",
                "white_tel": {},
                "black_tel": {},
            },
        ]
        rows = standings(games)
        self.assertEqual(rows[0]["name"], "Mujrim Obsidian")
        self.assertEqual(rows[0]["points"], 2.5)
        self.assertEqual(h2h(games, "Mujrim Obsidian", "Obsidian")["score_pct"], 75.0)
        finished = standings(finished_only(games))
        obs = next(row for row in finished if row["name"] == "Mujrim Obsidian")
        self.assertEqual(obs["points"], 1.5)

    def test_prefer_richer_source(self) -> None:
        disk = [{"white": "A", "black": "B", "white_score": 1.0}] * 3
        sqlite = [{"white": "A", "black": "B", "white_score": 1.0}]
        self.assertEqual(len(prefer_games(disk, sqlite)), 3)
        self.assertEqual(len(prefer_games([], sqlite)), 1)


if __name__ == "__main__":
    sys.exit(main())

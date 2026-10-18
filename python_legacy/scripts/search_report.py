"""Summarize a broad-search archive. Usage: python3 python_legacy/scripts/search_report.py data/search/broad_v1.json [top]"""
import json
import sys


def fp(s, t):
    v = s.get(str(t)) or {}
    return v.get("YIELD_FOOD", 0), v.get("YIELD_PRODUCTION", 0)


def main():
    d = json.load(open(sys.argv[1]))
    top = int(sys.argv[2]) if len(sys.argv) > 2 else 6
    for obj, cells in d.items():
        print(f"=== {obj}: {len(cells)} strategy cells")
        for c in cells[:top]:
            f30, p30 = fp(c["snaps"], 30)
            f60, p60 = fp(c["snaps"], 60)
            s = c["summary"]
            settles = [t for t, city, item in s["production"] if item == "UNIT_SETTLER"]
            cap = s["cities"][0][0]
            cap_order = [item.split("_", 1)[1] for t, city, item in s["production"] if city == cap][:12]
            print(f"- value {c['value']:.1f}  cell {c['cell']}  T30 F{f30:.0f}/P{p30:.0f}  T60 F{f60:.0f}/P{p60:.0f}  "
                  f"cities@30 {c['desc'].get('cities@30')} @60 {c['desc'].get('cities@60')}  districts@60 {c['desc'].get('districts@60')}")
            print(f"    settlers started T{settles[:10]}")
            print(f"    capital: {' > '.join(cap_order)}")
            print(f"    research: {' > '.join(r[5:] for r in s['research'][:10])}")
            print(f"    civics: {' > '.join(r[6:] for r in s['civics'][:6])}  pantheon {s['pantheon']}  gov {s['government']}")


if __name__ == "__main__":
    main()

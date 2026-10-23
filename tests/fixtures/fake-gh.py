import json, os, sys

args = sys.argv[1:]
d = os.environ["FAKE_GH_DIR"]
with open(os.environ["FAKE_GH_LOG"], "a") as f:
    f.write(" ".join(args) + "\n")
if args[:2] == ["auth", "status"]:
    sys.exit(int(os.environ.get("FAKE_GH_AUTH", "0")))

path = os.path.join(d, "issues.json")
clock = os.path.join(d, "clock")
issues = json.load(open(path))


def save():
    json.dump(issues, open(path, "w"), indent=1)


def bump(i):
    c = int(open(clock).read()) + 1 if os.path.exists(clock) else 1
    open(clock, "w").write(str(c))
    i["updatedAt"] = "2026-01-01T00:%02d:%02dZ" % (c // 60, c % 60)


def opt(name, default=None):
    return args[args.index(name) + 1] if name in args else default


def find(n):
    for i in issues:
        if i["number"] == n:
            return i
    sys.stderr.write("GraphQL: Could not resolve to an issue with the number of %d.\n" % n)
    sys.exit(1)


cmd = args[:2]
if cmd == ["issue", "list"]:
    limit = int(opt("--limit", "30"))
    print(json.dumps([i for i in issues if i["state"] == "OPEN"][:limit]))
elif cmd == ["issue", "view"]:
    print(json.dumps(find(int(args[2]))))
elif cmd == ["issue", "edit"]:
    i = find(int(args[2]))
    i["title"] = opt("--title", i["title"])
    bump(i)
    save()
elif cmd in (["issue", "close"], ["issue", "reopen"]):
    i = find(int(args[2]))
    i["state"] = "CLOSED" if cmd[1] == "close" else "OPEN"
    bump(i)
    save()
else:
    sys.stderr.write("fake gh: unsupported %s\n" % args)
    sys.exit(2)

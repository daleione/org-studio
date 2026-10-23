"""Org Babel process bridge. User values travel as typed JSON, never as code."""
import decimal
import json
import math
import sys


def decode(item):
    kind = item["type"]
    value = item.get("value")
    if kind == "nil":
        return None
    if kind == "integer":
        return int(value)
    if kind == "decimal":
        return decimal.Decimal(value)
    if kind == "list":
        return [decode(v) for v in value]
    if kind == "hline":
        return "hline"
    return value


def encode(value, depth=0):
    if depth > 32:
        raise ValueError("Babel return value is nested too deeply")
    if value is None:
        return {"type": "nil"}
    if isinstance(value, bool):
        kind = "bool"
    elif isinstance(value, int):
        kind, value = "integer", str(value)
    elif isinstance(value, decimal.Decimal):
        if not value.is_finite():
            raise ValueError("Babel numbers must be finite")
        kind, value = "decimal", str(value)
    elif isinstance(value, float):
        if not math.isfinite(value):
            raise ValueError("Babel numbers must be finite")
        kind = "float"
    elif isinstance(value, str):
        kind = "text"
    elif isinstance(value, (list, tuple)):
        kind, value = "list", [encode(v, depth + 1) for v in value]
    else:
        raise TypeError("Return a scalar, list, or tuple; convert other objects explicitly")
    return {"type": kind, "value": value}


request = json.load(sys.stdin)
variables = {name: decode(value) for name, value in request["variables"]}
namespace = {"__name__": "__main__"}
namespace.update(variables)
source = request["source"]
if request["value_result"]:
    # Like ob-python, value results come from an explicit return in a function.
    wrapped = "def __org_babel_value__(" + ", ".join(variables) + "):\n" + "".join(
        "    " + line + "\n" for line in source.splitlines()
    ) + "    pass\n"
    exec(compile(wrapped, "<org-babel>", "exec"), namespace)
    value = namespace["__org_babel_value__"](**variables)
    reply = json.dumps(encode(value), ensure_ascii=False, allow_nan=False)
    if len(reply.encode("utf-8")) > 1024 * 1024:
        raise ValueError("Python return value exceeds 1 MiB")
    with open(sys.argv[1], "w", encoding="utf-8") as output:
        output.write(reply)
else:
    exec(compile(source, "<org-babel>", "exec"), namespace)

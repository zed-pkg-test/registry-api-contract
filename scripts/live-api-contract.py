#!/usr/bin/env python3
import json,os,sys,urllib.request
from pathlib import Path
base=os.getenv("TARGET_API_URL")
if not base: print("blocked: TARGET_API_URL required",file=sys.stderr); raise SystemExit(78)
e=[]
for p in [x.strip() for x in os.getenv("API_HEALTH_PATHS","/healthz,/readyz").split(",") if x.strip()]:
    u=base.rstrip("/")+"/"+p.lstrip("/")
    with urllib.request.urlopen(u,timeout=20) as r: e.append({"url":u,"status":r.status})
out=Path("artifacts/api-contract.json"); out.parent.mkdir(exist_ok=True); out.write_text(json.dumps(e,indent=2)+"\n")

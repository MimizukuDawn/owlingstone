#!/usr/bin/env bash
# Smoke test against `wrangler dev --local` (see server/README.md). Usage: test/smoke.sh http://localhost:8799
set -euo pipefail
API=${1:-http://localhost:8799}
ID=$(python3 -c "import uuid;print(uuid.uuid4())")
GAME='{"v":1,"app":"1.8.0","mode":"solo","hero":"Arch-Villain Rafaam","deity":null,"comp":"undead-apm","races":["UNDEAD","PIRATE"],"place":3,
 "board":[{"id":"BG25_008","a":12,"h":10,"g":false}],
 "fights":[{"t":5,"tier":3,"otier":3,"hp":30,"r":"W","d":4,"me":[{"id":"BG25_008","a":4,"h":2,"g":false}],"opp":[]}],"turns":6,"actions":[2,1,6,8,5,2],"end_hp":-2,"conceded":false}'
code() { curl -s -o /dev/null -w "%{http_code}" "$@"; }
check() { [ "$1" = "$2" ] && echo "ok   $3" || { echo "FAIL $3 (got $1, want $2)"; exit 1; }; }
H=(-H "content-type: application/json")
check "$(code "${H[@]}" -d "{\"install\":\"$ID\",\"games\":[$GAME]}" "$API/v1/games")" 200 "store one game"
check "$(code "${H[@]}" -d "{\"install\":\"$ID\",\"games\":[$GAME,$GAME]}" "$API/v1/games")" 200 "store a batch"
check "$(code "${H[@]}" -d "{\"install\":\"not-a-uuid\",\"games\":[$GAME]}" "$API/v1/games")" 400 "bad install id"
BAD=${GAME/\"place\":3/\"place\":9}
check "$(code "${H[@]}" -d "{\"install\":\"$ID\",\"games\":[$BAD]}" "$API/v1/games")" 422 "impossible placement"
EXTRA=${GAME/\"v\":1,/\"v\":1,\"battletag\":\"Someone#1234\",}
OUT=$(curl -s "${H[@]}" -d "{\"install\":\"$ID\",\"games\":[$EXTRA]}" "$API/v1/games"); echo "     extra field accepted but dropped: $OUT"
check "$(code -H 'content-type: text/plain' -d x "$API/v1/games")" 415 "json only"
BIG=$(python3 -c "print('{\"install\":\"$ID\",\"games\":[' + ','.join(['$(echo $GAME | tr -d '\n' | sed 's/"/\\"/g')']*1) + '],\"pad\":\"' + 'x'*70000 + '\"}')")
check "$(code "${H[@]}" --data-binary "$BIG" "$API/v1/games")" 413 "body over 64 KB"
for i in $(seq 1 12); do curl -s -o /dev/null "${H[@]}" -d "{\"install\":\"$ID\",\"games\":[$GAME,$GAME,$GAME]}" "$API/v1/games"; done
check "$(code "${H[@]}" -d "{\"install\":\"$ID\",\"games\":[$GAME,$GAME,$GAME]}" "$API/v1/games")" 429 "40 games a day per install"
check "$(code -X DELETE "$API/v1/installs/$ID")" 200 "delete my games"
check "$(code "$API/nothing")" 404 "unknown route"
# Feedback
ID2=$(python3 -c "import uuid;print(uuid.uuid4())")
check "$(code "${H[@]}" -d "{\"install\":\"$ID2\",\"games\":[],\"feedback\":[{\"kind\":\"idea\",\"text\":\"Dark mode please\",\"app\":\"1.8.0\"}]}" "$API/v1/games")" 200 "feedback alone"
check "$(code "${H[@]}" -d "{\"install\":\"$ID2\",\"games\":[],\"feedback\":[{\"kind\":\"spam\",\"text\":\"x\"}]}" "$API/v1/games")" 422 "bad feedback kind"
check "$(code "$API/v1/admin/feedback")" 401 "admin needs the key"
check "$(code -H "authorization: Bearer wrong" "$API/v1/admin/feedback")" 401 "wrong key refused"
KEY=${ADMIN_KEY:-$(sed -n "s/^ADMIN_KEY=//p" "$(dirname "$0")/../.dev.vars")}
LIST=$(curl -s -H "authorization: Bearer $KEY" "$API/v1/admin/feedback"); echo "     admin list: $LIST"
FID=$(echo "$LIST" | python3 -c "import json,sys;print(json.load(sys.stdin)['feedback'][-1]['id'])")
check "$(code -H "authorization: Bearer $KEY" "${H[@]}" -d "{\"ids\":[$FID]}" "$API/v1/admin/feedback/done")" 200 "mark handled"
check "$(code -X DELETE "$API/v1/installs/$ID2")" 200 "delete my feedback"

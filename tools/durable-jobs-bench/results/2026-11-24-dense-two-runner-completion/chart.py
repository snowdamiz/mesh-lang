"""Create a standalone chart of the five qualified native stage comparisons."""
import json
from pathlib import Path
from statistics import median
import sys

HERE = Path(__file__).resolve().parent


def chart(records):
    series = {}
    for block, lanes in sorted(records.items()):
        series[block[:2]] = {name: [values["effect_committed_us"], values["settled_us"]]
                             for name, values in lanes.items()}
    series["median"] = {lane: [median(block[lane][i] for block in series.values()) for i in range(2)]
                        for lane in next(iter(series.values()))}
    template = """<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Dense queue drain on the Linode</title>
<style>
*{box-sizing:border-box}body{margin:0;color:var(--foreground,#17232d);font:15px/1.45 Inter,system-ui,sans-serif}
section{margin:8px 0 20px}h2{font-size:21px;line-height:1.3;margin:0 0 5px;font-weight:650}
p{margin:0 0 12px}label,select{font:inherit}select{background:var(--card,white);color:inherit;border:1px solid var(--border,#aaa);border-radius:5px;padding:4px}
.row{margin-top:14px}.row>div:first-child{display:flex;justify-content:space-between;gap:8px;align-items:baseline}
.name{font-weight:600}.num{font:13px/1.4 'IBM Plex Mono',ui-monospace,monospace;font-variant-numeric:tabular-nums}
.track{height:11px;border-radius:3px;margin-top:4px;background:var(--muted,#e7ebee);overflow:hidden;display:flex}
.work,.tail{display:inline-block;height:100%}.work,.effect{background:#237f88}.tail,.settle{background:#d38940}
.legend{display:flex;flex-wrap:wrap;gap:3px 15px;margin:10px 0;color:var(--muted-foreground,#53606b)}
.key{display:inline-block;width:9px;height:9px;margin-right:5px;border-radius:2px}
.note{color:var(--muted-foreground,#53606b);font-size:13px;margin:12px 0 0}
</style><section aria-label="Dense benchmark stage timings">
<h2>When did the last application write commit?</h2>
<p>1,024 dense jobs on two Linode CPUs. Each bar starts at enqueue commit.</p>
<label>Rotation <select id="rot"><option value="median">Five-run median</option><option>01</option><option>02</option><option>03</option><option>04</option><option>05</option></select></label>
<div class="legend"><span><i class="key effect"></i>Through last application write</span><span><i class="key settle"></i>Through final settlement</span></div>
<div id="rows" aria-live="polite"></div>
<p class="note">Mesh used two 32-slot runners. Each peer used eight 8-slot runners. The two displayed times are measured from enqueue commit.</p>
</section><script>
const data=DATA_JSON;
const names={"mesh-v1-2x32":"Mesh V1","oban-8x8":"Elixir · Oban","river-8x8":"Go · River","apalis-8x8":"Rust · Apalis"};
const format=new Intl.NumberFormat("en-US",{maximumFractionDigits:0});
const max=Math.ceil(Math.max(...Object.values(data).flatMap(s=>Object.values(s).map(w=>w[1])))/100)*100;
function draw(){
 const parent=document.getElementById("rows");parent.replaceChildren();
 for(const [k,name] of Object.entries(names)){
  const [w,t]=data[document.getElementById("rot").value][k];
  const row=document.createElement("div");row.className="row";
  const line=document.createElement("div"),label=document.createElement("span"),num=document.createElement("span");
  label.className="name";label.textContent=name;num.className="num";num.textContent=format.format(w)+" / "+format.format(t)+" ms";
  line.append(label,num);
  const track=document.createElement("div");track.className="track";track.setAttribute("aria-hidden","true");
  for(const [cls,value] of [["work",w],["tail",t-w]]){const seg=document.createElement("i");seg.className=cls;seg.style.width=(100*value/max)+"%";track.append(seg);}
  row.append(line,track);parent.append(row);
 }
}
document.getElementById("rot").addEventListener("change",draw);draw();
</script></html>"""
    return template.replace("DATA_JSON", json.dumps(series, sort_keys=True, separators=(",", ":")))


if __name__ == "__main__":
    source = Path(sys.argv[1]) if len(sys.argv) == 2 else HERE / "results.json"
    print(chart(json.loads(source.read_bytes())["native"]))

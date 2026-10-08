# python3 sheet.py <prefix> <cols> <out.png>: tile stills/<prefix>*.png in time order
import glob, re, subprocess, sys
prefix, cols, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
files = sorted(glob.glob(f"stills/{prefix}*.png"), key=lambda f: float(re.findall(r"[\d.]+(?=\.png)", f)[0]))
rows = -(-len(files) // cols)
args, parts = [], []
for i, f in enumerate(files):
    args += ["-i", f]
    parts.append(f"[{i}]scale=640:-1,pad=648:368:4:4:0x333333[p{i}]")
for i in range(len(files), rows * cols):
    args += ["-f", "lavfi", "-i", "color=c=0x333333:s=648x368:d=1"]
    parts.append(f"[{i}]null[p{i}]")
rowl = [f"{''.join(f'[p{r * cols + c}]' for c in range(cols))}hstack=inputs={cols}[r{r}]" for r in range(rows)]
fc = ";".join(parts + rowl + ([f"{''.join(f'[r{r}]' for r in range(rows))}vstack=inputs={rows}"] if rows > 1 else [])).replace("[r0]", "") if rows == 1 else ";".join(parts + rowl + [f"{''.join(f'[r{r}]' for r in range(rows))}vstack=inputs={rows}"])
subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", *args, "-filter_complex", fc, "-frames:v", "1", out], check=True)
print(out, [re.findall(r"[\d.]+(?=\.png)", f)[0] for f in files])

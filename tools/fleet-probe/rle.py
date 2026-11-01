import subprocess, sys
# usage: rle.py image geometry(e.g. 1x1120+20+0)  -> prints runs of (start,len,hex)
img, geo = sys.argv[1], sys.argv[2]
out = subprocess.run(["magick", img, "-crop", geo, "+repage", "-depth", "8", "txt:-"], capture_output=True, text=True).stdout
runs = []
for line in out.splitlines()[1:]:
    pos, rest = line.split(":",1)
    x, y = map(int, pos.split(","))
    hexv = rest.split("#")[1].split()[0][:6]
    i = x if y == 0 else y
    if runs and runs[-1][2] == hexv: runs[-1][1] += 1
    else: runs.append([i, 1, hexv])
for s, l, h in runs:
    if l >= int(sys.argv[3]) if len(sys.argv) > 3 else 1: print(f"{s:5d} len {l:4d} #{h}")

import subprocess,sys
def req10(rev):
    out=subprocess.run(['python3','scripts/production-lines.py',rev],capture_output=True,text=True,check=True).stdout
    n=0
    for l in out.splitlines():
        p=l.split()
        if len(p)==2 and p[1] in ('main','dispatch','output'): n+=int(p[0])
    return n
for a in sys.argv[1:]:
    if '=' in a:
        k,m=a.split('=')
        print(k, req10(m)-req10(m+'^1'))
    else: print(a, req10(a))

import sys, zipfile, re
from xml.etree import ElementTree as ET
NS='{http://schemas.openxmlformats.org/spreadsheetml/2006/main}'
RNS='{http://schemas.openxmlformats.org/officeDocument/2006/relationships}'
z=zipfile.ZipFile(sys.argv[1])
shared=[]
if 'xl/sharedStrings.xml' in z.namelist():
    for si in ET.fromstring(z.read('xl/sharedStrings.xml')):
        shared.append(''.join(t.text or '' for t in si.iter(NS+'t')))
wb=ET.fromstring(z.read('xl/workbook.xml'))
rels={r.get('Id'):r.get('Target') for r in ET.fromstring(z.read('xl/_rels/workbook.xml.rels'))}
def colnum(ref):
    s=re.match(r'[A-Z]+',ref).group(0); n=0
    for c in s: n=n*26+ord(c)-64
    return n-1
for sh in wb.find(NS+'sheets'):
    name=sh.get('name'); tgt=rels[sh.get(RNS+'id')].lstrip('/')
    if not tgt.startswith('xl/'): tgt='xl/'+tgt
    print('===SHEET=== '+name)
    root=ET.fromstring(z.read(tgt))
    for row in root.iter(NS+'row'):
        cells={}
        for c in row.findall(NS+'c'):
            v=c.find(NS+'v'); t=c.get('t')
            if t=='inlineStr':
                val=''.join(x.text or '' for x in c.iter(NS+'t'))
            elif v is None: val=''
            elif t=='s': val=shared[int(v.text)]
            else: val=v.text
            cells[colnum(c.get('r'))]=(val or '').replace('\n',' ').replace('\t',' ')
        if cells:
            print('\t'.join(cells.get(i,'') for i in range(max(cells)+1)))

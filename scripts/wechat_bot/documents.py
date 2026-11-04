"""Bounded content extraction; documents are data and are never opened as programs."""
import csv
import io
import json
from pathlib import Path
import posixpath
import zipfile
import xml.etree.ElementTree as ET

MAX_BYTES=10*1024*1024
MAX_TEXT=16000


def text_decode(data):
    for encoding in ['utf-8-sig','gb18030']:
        try:
            text=data.decode(encoding)
            if '\x00' in text: raise ValueError('binary content is not a text document')
            return text
        except UnicodeDecodeError:
            continue
    raise ValueError('unsupported text encoding')


def xml(archive,name):
    info=archive.getinfo(name)
    if info.file_size>2*1024*1024: raise ValueError('XML part too large')
    data=archive.read(info)
    if b'<!DOCTYPE' in data.upper() or b'<!ENTITY' in data.upper():
        raise ValueError('document XML entities are not accepted')
    return ET.fromstring(data)


def extract(path,extension,remote=None):
    path=Path(path)
    if not path.is_file() or path.stat().st_size>MAX_BYTES:
        raise ValueError('file size limit')
    extension=extension.lower()
    if extension=='pdf':
        if not path.read_bytes().startswith(b'%PDF-') or not isinstance(remote,dict):
            raise ValueError('PDF extraction unavailable')
        return {'text':remote.get('text','')[:MAX_TEXT],'page_count':remote.get('page_count'),
                'truncated':bool(remote.get('truncated')),'scanned_or_empty':bool(remote.get('scanned_or_empty'))}
    if extension in ['txt','md','csv','json']:
        text=text_decode(path.read_bytes())
        if extension=='json':
            text=json.dumps(json.loads(text),ensure_ascii=False,indent=2)
        if extension=='csv':
            rows=[]
            for index,row in enumerate(csv.reader(io.StringIO(text))):
                if index>=100: break
                rows.append(row[:30])
            return {'rows':rows,'truncated':index>=100 if text else False}
        return {'text':text[:MAX_TEXT],'truncated':len(text)>MAX_TEXT}
    if extension not in ['docx','xlsx']:
        raise ValueError('unsupported document type')
    with zipfile.ZipFile(path) as archive:
        infos=archive.infolist()
        if len(infos)>10000 or sum(i.file_size for i in infos)>20*1024*1024:
            raise ValueError('expanded document size limit')
        if any(i.flag_bits&1 for i in infos): raise ValueError('encrypted document unsupported')
        if extension=='docx':
            root=xml(archive,'word/document.xml')
            ns={'w':'http://schemas.openxmlformats.org/wordprocessingml/2006/main'}
            paragraphs=[''.join(t.text or '' for t in p.findall('.//w:t',ns)) for p in root.findall('.//w:p',ns)]
            text='\n'.join(paragraphs)
            return {'text':text[:MAX_TEXT],'truncated':len(text)>MAX_TEXT,'embedded_images_read':False}
        ns={'s':'http://schemas.openxmlformats.org/spreadsheetml/2006/main'}
        strings=[]
        if 'xl/sharedStrings.xml' in archive.namelist():
            root=xml(archive,'xl/sharedStrings.xml')
            strings=[''.join(t.text or '' for t in item.findall('.//s:t',ns)) for item in root.findall('s:si',ns)]
        relationships=xml(archive,'xl/_rels/workbook.xml.rels')
        targets={r.attrib['Id']:r.attrib['Target'] for r in relationships if r.attrib.get('TargetMode')!='External'}
        workbook=xml(archive,'xl/workbook.xml')
        sheets=[]
        for sheet in workbook.findall('s:sheets/s:sheet',ns)[:3]:
            relation=sheet.attrib['{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id']
            target=targets[relation]
            name=posixpath.normpath(target.lstrip('/') if target.startswith('/') else posixpath.join('xl',target))
            if not name.startswith('xl/') or '/..' in name: raise ValueError('invalid worksheet target')
            root=xml(archive,name)
            rows=[]
            all_rows=root.findall('s:sheetData/s:row',ns)
            for row in all_rows[:100]:
                cells=[]
                for cell in row.findall('s:c',ns)[:30]:
                    value=cell.findtext('s:v',default='',namespaces=ns)
                    if cell.attrib.get('t')=='s': value=strings[int(value)]
                    elif cell.attrib.get('t')=='inlineStr': value=''.join(t.text or '' for t in cell.findall('.//s:t',ns))
                    elif cell.find('s:f',ns) is not None and not value: value='[公式无缓存值，未执行]'
                    cells.append({'cell':cell.attrib.get('r'),'value':value})
                rows.append(cells)
            sheets.append({'name':sheet.attrib.get('name'),'rows':rows,'truncated':len(all_rows)>100})
        return {'sheets':sheets,'truncated':len(workbook.findall('s:sheets/s:sheet',ns))>3,'formulas_executed':False}

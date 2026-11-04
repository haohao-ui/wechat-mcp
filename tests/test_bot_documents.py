import asyncio
import base64
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from scripts.wechat_bot.documents import extract
from scripts.wechat_bot.native_capture import NativeCapture
from scripts.agent_bridge.codex import private_directory


class DocumentTests(unittest.TestCase):
    def setUp(self): self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name)
    def tearDown(self): self.temp.cleanup()

    def test_text_and_csv_contents_are_actually_read(self):
        p=self.root/'a.txt';p.write_text('口令：竹叶-47',encoding='utf-8')
        self.assertIn('竹叶-47',extract(p,'txt')['text'])
        p.write_text('名称,数量\n苹果,3\n',encoding='utf-8')
        self.assertEqual(extract(p,'csv')['rows'][1],['苹果','3'])

    def test_binary_disguised_as_text_rejected(self):
        p=self.root/'a.txt';p.write_bytes(b'bad\x00binary')
        with self.assertRaises(ValueError): extract(p,'txt')

    def test_docx_reads_document_xml(self):
        p=self.root/'a.docx'
        with zipfile.ZipFile(p,'w') as z:
            z.writestr('word/document.xml','<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>实际正文 竹叶-47</w:t></w:r></w:p></w:body></w:document>')
        self.assertIn('实际正文 竹叶-47',extract(p,'docx')['text'])

    def test_xml_entities_rejected(self):
        p=self.root/'a.docx'
        with zipfile.ZipFile(p,'w') as z:
            z.writestr('word/document.xml','<!DOCTYPE x [<!ENTITY e "bad">]><x>&e;</x>')
        with self.assertRaises(ValueError): extract(p,'docx')

    def test_xlsx_reads_cells_without_executing_formulas(self):
        p=self.root/'a.xlsx'
        with zipfile.ZipFile(p,'w') as z:
            z.writestr('xl/workbook.xml','<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" r:id="r1"/></sheets></workbook>')
            z.writestr('xl/_rels/workbook.xml.rels','<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Target="worksheets/sheet1.xml"/></Relationships>')
            z.writestr('xl/worksheets/sheet1.xml','<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row><c r="A1" t="inlineStr"><is><t>竹叶-47</t></is></c><c r="B1"><f>HYPERLINK("https://example.invalid")</f></c></row></sheetData></worksheet>')
        result=extract(p,'xlsx')
        self.assertEqual(result['sheets'][0]['rows'][0][0]['value'],'竹叶-47')
        self.assertFalse(result['formulas_executed'])

    def test_pdf_requires_original_header_and_actual_extraction(self):
        p=self.root/'a.pdf';p.write_bytes(b'%PDF-1.7\nfixture')
        with self.assertRaises(ValueError): extract(p,'pdf')
        self.assertEqual(extract(p,'pdf',{'text':'真实提取','page_count':1})['text'],'真实提取')

    def test_oversized_zip_expansion_rejected(self):
        p=self.root/'a.docx'
        with zipfile.ZipFile(p,'w',zipfile.ZIP_DEFLATED) as z:
            z.writestr('word/document.xml','x'*(21*1024*1024))
        with self.assertRaisesRegex(ValueError,'expanded'): extract(p,'docx')


class MediaTransportTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=private_directory(Path(self.temp.name)/'private')
        self.capture=NativeCapture('fixture',None,'示例联系人甲',self.root,'a'*64)
    def tearDown(self): self.temp.cleanup()

    async def test_chunked_bytes_digest_and_extension(self):
        data=b'actual original file bytes'*10000
        async def call(operation,**params):
            offset=params['offset'];chunk=data[offset:offset+65536]
            return {'offset':offset,'size':len(data),'bytes':base64.b64encode(chunk).decode(),
                    'eof':offset+len(chunk)==len(data),'sha256':hashlib.sha256(data).hexdigest(),'extension':'txt'}
        self.capture.call=call
        p=await self.capture.download('b'*64,kind='file')
        self.assertEqual(p.read_bytes(),data)
        self.assertEqual(p.stat().st_mode&0o777,0o600)

    async def test_corrupt_transfer_never_produces_attachment(self):
        async def call(*_,**__):
            return {'offset':0,'size':3,'bytes':'YmFk','eof':True,'sha256':'0'*64,'extension':'txt'}
        self.capture.call=call
        with self.assertRaisesRegex(RuntimeError,'digest mismatch'):
            await self.capture.download('c'*64,kind='file')

    async def test_media_path_cannot_escape_private_directory(self):
        with self.assertRaises(ValueError): await self.capture.download('../escape')


if __name__=='__main__':unittest.main()

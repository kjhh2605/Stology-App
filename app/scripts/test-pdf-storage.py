#!/usr/bin/env python3
"""Independent executable acceptance for PDF storage and analysis boundaries.

Uses temporary STOLOGY_DATA_DIR roots and no connected analysis provider.
Generated fixtures are deleted at exit, including only Trash paths returned for
these fixture IDs. It never changes HOME or opens the user's normal data root.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import zipfile


HELPER_SOURCE = r'''
use std::{io::Read, path::{Path, PathBuf}};
use serde_json::{json, Value};
use stology_core::pdf_recovery;
fn main() {
    let mut input=String::new();std::io::stdin().read_to_string(&mut input).unwrap();
    let v:Value=serde_json::from_str(&input).unwrap();
    let root=PathBuf::from(v["root"].as_str().unwrap());let id=v["id"].as_str().unwrap();
    let result=(|| -> Result<Value,Box<dyn std::error::Error>> {
        Ok(match v["op"].as_str().unwrap() {
            "open"=>{let s=pdf_recovery::open_session(&root,id)?;
                json!({"document":s.document,"path":s.path,"hash":s.hash,"version":s.version,
                    "pdf_seq":s.pdf_seq,"journal_seq":s.journal_seq,
                    "commands":s.commands.into_iter().map(|c|json!({"seq":c.seq,"command":c.command})).collect::<Vec<_>>()})},
            "append"=>json!({"journal_seq":pdf_recovery::append(&root,id,v["seq"].as_u64().unwrap(),&v["command"])?}),
            "publish"=>{let p=pdf_recovery::publish(&root,id,v["version"].as_i64().unwrap(),v["hash"].as_str().unwrap(),v["through_seq"].as_u64().unwrap(),Path::new(v["temp_path"].as_str().unwrap()))?;
                json!({"hash":p.hash,"version":p.version,"pdf_seq":p.pdf_seq,"path":p.path})},
            _=>panic!("unknown helper op"),
        })
    })();
    match result {Ok(result)=>println!("{}",json!({"ok":true,"result":result})),
        Err(error)=>println!("{}",json!({"ok":false,"error":error.to_string()}))}
}
'''


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class Acceptance:
    def __init__(self, binary, root):
        self.binary = binary
        self.root = root
        self.results = []
        self.trash = []
        self.env = dict(os.environ, STOLOGY_DATA_DIR=str(root))

    def call(self, op, fail=False, **fields):
        proc = subprocess.run(
            [str(self.binary), "--root", str(self.root), "--request"],
            input=json.dumps(dict(op=op, **fields)), text=True,
            capture_output=True, env=self.env, timeout=35,
        )
        try:
            response = json.loads(proc.stdout)
        except ValueError as error:
            raise AssertionError((op, proc.returncode, proc.stdout, proc.stderr)) from error
        assert response.get("ok") is (not fail), (op, response, proc.stderr)
        if fail:
            return response
        result = response["result"]
        if result.get("trash_path"):
            self.trash.append(Path(result["trash_path"]))
        return result

    def passed(self, case):
        self.results.append({"case": case, "result": "pass"})

    def versions(self):
        return {p.name: sha(p) for p in self.root.joinpath("versions").glob("*") if p.is_file()}

    def analysis_state(self, workspace, document_id):
        entries = self.call("tree", workspace_id=workspace)["entries"]
        return next(entry["analysis_state"] for entry in entries if entry["id"] == document_id)

    def direct(self, op, fail=False, **fields):
        proc = subprocess.run([str(self.helper)], input=json.dumps(dict(op=op, root=str(self.root), **fields)),
                              text=True, capture_output=True, timeout=35, env=self.env)
        response = json.loads(proc.stdout)
        assert response["ok"] is (not fail), (op, response, proc.stderr)
        return response if fail else response["result"]

    def prepare(self, session, marker):
        path = Path(session["path"]).parent / ".acceptance-pending.pdf"
        path.write_bytes(Path(session["path"]).read_bytes() + f"\n% {marker}\n".encode())
        return path

    def publish(self, session, through, temp_path, fail=False):
        return self.direct("publish", id=session["document"]["id"], version=session["version"],
                           hash=session["hash"], through_seq=through, temp_path=str(temp_path), fail=fail)

    def import_pdf(self, fixtures, marker):
        workspace = self.call("workspace.create", name=marker)["id"]
        return self.call("material.import", workspace_id=workspace, path=str(fixtures / "biology.pdf"))

    def run_recovery(self, fixtures):
        doc = self.import_pdf(fixtures, "Recovery ordering")
        session = self.direct("open", id=doc["id"])
        command = {"id": "annotation-1", "after": {"type": "box"}}
        self.direct("append", id=doc["id"], seq=1, command=command)
        self.direct("append", id=doc["id"], seq=1, command=command)
        self.direct("append", id=doc["id"], seq=1, command={"different": True}, fail=True)
        self.direct("append", id=doc["id"], seq=3, command=command, fail=True)
        self.direct("append", id=doc["id"], seq=2, command={"id": "annotation-1", "after": {"type": "moved"}})
        pending = self.direct("open", id=doc["id"])
        assert [c["seq"] for c in pending["commands"]] == [1, 2]
        versions = self.versions()
        result = self.publish(session, 1, self.prepare(session, "first publication"))
        assert result["version"] == session["version"] + 1
        reopened = self.direct("open", id=doc["id"])
        assert reopened["pdf_seq"] == 1 and reopened["journal_seq"] == 2
        assert [c["seq"] for c in reopened["commands"]] == [2]
        self.direct("append", id=doc["id"], seq=1, command=command)
        self.publish(reopened, 2, self.prepare(reopened, "second publication"))
        settled = self.direct("open", id=doc["id"])
        assert settled["pdf_seq"] == settled["journal_seq"] == 2 and not settled["commands"]
        assert self.versions() == versions
        self.passed("cross-process ordered journal, retry dedup, partial publication and GC preserve later edits")

        old_path = Path(settled["path"])
        renamed = self.call("note.rename", id=doc["id"], title="Renamed annotation", expected_version=settled["version"])
        self.direct("append", id=doc["id"], seq=3, command=command)
        self.publish(settled, 3, self.prepare(dict(settled, path=renamed["absolute_path"]), "late old-version save"), fail=True)
        assert not old_path.exists()
        current = self.direct("open", id=doc["id"])
        self.publish(current, 3, self.prepare(current, "after rename"))
        self.passed("renamed document rejects obsolete publication without recreating old path")

        conflict = self.direct("open", id=doc["id"])
        self.direct("append", id=doc["id"], seq=4, command=command)
        temp = self.prepare(conflict, "unpublished user annotation")
        Path(conflict["path"]).write_bytes(Path(conflict["path"]).read_bytes() + b"\n% external concurrent edit\n")
        external_hash = sha(conflict["path"])
        self.publish(conflict, 4, temp, fail=True)
        self.call("source.get", id=doc["id"], fail=True)
        assert sha(conflict["path"]) == external_hash
        with sqlite3.connect(self.root / "pdf-journal.sqlite") as db:
            assert db.execute("SELECT count(*) FROM commands WHERE document_id=?", (doc["id"],)).fetchone()[0] == 1
        self.call("material.delete", id=doc["id"])
        self.publish(conflict, 4, temp, fail=True)
        assert not Path(conflict["path"]).exists()
        self.passed("external edit conflict retains journal; deleted PDF is never resurrected by late save")

        for point in ("before-rename", "after-rename", "corrupt-temp"):
            doc = self.import_pdf(fixtures, "Crash " + point)
            session = self.direct("open", id=doc["id"])
            self.direct("append", id=doc["id"], seq=1, command=command)
            temp = self.prepare(session, point)
            target_hash = sha(temp)
            def identity(path):
                stat = Path(path).stat()
                return {"dev": stat.st_dev, "ino": stat.st_ino, "len": stat.st_size,
                        "mtime": stat.st_mtime_ns // 1_000_000_000, "nanos": stat.st_mtime_ns % 1_000_000_000,
                        "ctime": stat.st_ctime_ns // 1_000_000_000, "cnanos": stat.st_ctime_ns % 1_000_000_000}
            intent = dict(id=doc["id"], path=session["path"], temp=str(temp), base_hash=session["hash"],
                          target_hash=target_hash, base_version=session["version"], through_seq=1,
                          base_identity=identity(session["path"]), target_identity=identity(temp))
            with sqlite3.connect(self.root / "storage.sqlite") as db:
                db.execute("INSERT INTO pdf_publish_intents VALUES(?,?)", (doc["id"], json.dumps(intent)))
            if point == "after-rename":
                os.replace(temp, session["path"])
            elif point == "corrupt-temp":
                temp.write_bytes(b"interrupted partial PDF")
            current = self.call("source.get", id=doc["id"])
            restored = self.direct("open", id=doc["id"])
            if point == "corrupt-temp":
                assert restored["pdf_seq"] == 0 and len(restored["commands"]) == 1
                assert current["hash"] == session["hash"]
            else:
                assert restored["pdf_seq"] == 1 and not restored["commands"]
                assert current["hash"] == target_hash
                assert current["version"] == session["version"] + 1
                assert self.call("source.get", id=doc["id"])["version"] == current["version"]
            self.passed("interrupted publication recovery: " + point)

    def run_generated(self, fixtures):
        workspace = self.call("workspace.create", name="Generated PDF acceptance")["id"]
        source_path = self.root / "강의자료.pptx"
        with zipfile.ZipFile(source_path, "w") as archive:
            archive.writestr("[Content_Types].xml", '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>')
        source = self.call("material.import", workspace_id=workspace, path=str(source_path))
        assert self.call("pdf.generated.get", source_id=source["id"])["document"] is None
        collision = Path(source["absolute_path"]).with_suffix(".pdf")
        collision.write_bytes((fixtures / "biology.pdf").read_bytes())
        collision_hash = sha(collision)
        versions = self.versions()
        generated = self.call("pdf.generated.register", source_id=source["id"], source_hash=source["hash"], pdf_path=str(fixtures / "biology.pdf"))
        assert generated["id"] != source["id"]
        assert generated["title"] == "강의자료 (2)"
        assert sha(collision) == collision_hash
        assert self.analysis_state(workspace, generated["id"]) == "unanalysed"
        assert self.versions() == versions
        reused = self.call("pdf.generated.register", source_id=source["id"], source_hash=source["hash"], pdf_path=str(fixtures / "biology.pdf"))
        assert reused["id"] == generated["id"]
        renamed = self.call("note.rename", id=generated["id"], title="필기 보존", expected_version=generated["version"])
        assert self.call("pdf.generated.get", source_id=source["id"])["document"]["absolute_path"] == renamed["absolute_path"]
        Path(source["absolute_path"]).write_bytes(source_path.read_bytes() + b"changed source")
        assert self.call("pdf.generated.get", source_id=source["id"])["document"]["id"] == generated["id"]
        assert sha(renamed["absolute_path"]) == generated["hash"]
        self.passed("PPTX PDF registration preserves filename collision, independent ID and link after rename/source change")
        current_source = self.call("source.get", id=source["id"])
        forced = self.call("pdf.generated.register", source_id=source["id"], source_hash=current_source["hash"], pdf_path=str(fixtures / "biology.pdf"), force_new=True)
        assert forced["id"] != generated["id"] and forced["hash"] == generated["hash"]
        self.call("material.delete", id=forced["id"])
        assert self.call("pdf.generated.get", source_id=source["id"])["document"] is None
        self.call("material.delete", id=source["id"])
        assert self.call("note.read", id=generated["id"])["id"] == generated["id"]
        assert Path(renamed["absolute_path"]).exists() and sha(collision) == collision_hash
        self.passed("identical conversion bytes can create independent PDF; deleting source or linked PDF preserves other files")

    def run_analysis_boundaries(self, fixtures):
        workspace = self.call("workspace.create", name="PDF analysis boundary acceptance")["id"]
        imported = []
        for filename in ("biology.md", "biology.pdf", "diagram.png"):
            doc = self.call("material.import", workspace_id=workspace, path=str(fixtures / filename))
            imported.append(doc)
            self.call("note.read", id=doc["id"])
            source = self.call("source.get", id=doc["id"])
            assert source.get("immutable_path") is None, source
        self.call("tree", workspace_id=workspace)
        self.call("graph.get", workspace_id=workspace)
        assert not self.versions()
        assert not list(self.root.glob("jobs/*"))
        assert all(self.analysis_state(workspace, doc["id"]) == "unanalysed" for doc in imported)
        self.passed("provider-free MD/PDF/image imports and all reads create no retained versions")

        note = imported[0]
        original_body = (fixtures / "biology.md").read_text()
        note = self.call("note.update", id=note["id"], body=original_body + "\nBefore analysis", expected_version=note["version"])
        self.call("note.read", id=note["id"])
        assert not self.versions()
        self.passed("Markdown edits before analysis do not retain versions")

        snapshot = self.call("source.snapshot", id=note["id"], expected_hash=note["hash"])
        assert sha(snapshot["immutable_path"]) == note["hash"]
        versions = self.versions()
        assert len(versions) == 1
        again = self.call("source.snapshot", id=note["id"], expected_hash=note["hash"])
        assert again["immutable_path"] == snapshot["immutable_path"]
        assert self.versions() == versions
        extraction = self.call("extract", path=snapshot["immutable_path"])
        unit = next(u for u in extraction["units"] if "광합성(photosynthesis)" in u["text"])
        result = {
            "concepts": [
                {"id": "a", "name": "엽록체", "aliases": ["chloroplast"], "definition": "광합성이 일어나는 세포 소기관"},
                {"id": "b", "name": "광합성", "aliases": ["photosynthesis"], "definition": "빛에너지를 화학 에너지로 바꾸는 과정"},
            ],
            "relations": [{"id": "r", "label": "수행한다", "aliases": [], "definition": "소기관에서 과정이 일어난다", "directed": True}],
            "assertions": [{"subject": "a", "relation": "r", "object": "b", "condition": "", "negated": False,
                "evidence": [{"locator": unit["locator"], "quote": "엽록체는 광합성이 일어나는 세포 소기관이다.", "image_path": None, "region": None}]}],
        }
        apply = dict(workspace_id=workspace, material_id=note["id"], input_hash=note["hash"],
                     provider="deterministic-fixture", model="no-network", result=result)
        assert self.call("graph.apply", **apply)["applied"]
        graph_before = self.call("graph.get", workspace_id=workspace)
        assert graph_before["sources"][0]["stale"] is False
        assert self.analysis_state(workspace, note["id"]) == "analysed"
        self.passed("explicit snapshot retained exactly once and supports immutable graph evidence")

        updated = self.call("note.update", id=note["id"], body=original_body + "\nChanged during analysis", expected_version=note["version"])
        graph_after = self.call("graph.get", workspace_id=workspace)
        assert graph_after["sources"][0]["stale"] is True
        assert self.analysis_state(workspace, note["id"]) == "stale"
        assert graph_after["assertions"]
        assert len(graph_after["assertions"]) == len(graph_before["assertions"])
        assert self.versions() == versions
        assert sha(snapshot["immutable_path"]) == note["hash"]
        self.call("graph.apply", fail=True, **apply)
        assert self.call("graph.get", workspace_id=workspace) == graph_after
        self.call("source.snapshot", id=note["id"], expected_hash=note["hash"], fail=True)
        assert self.versions() == versions
        latest = self.call("source.snapshot", id=note["id"], expected_hash=updated["hash"])
        assert latest["immutable_path"] != snapshot["immutable_path"]
        assert len(self.versions()) == 2
        self.passed("late analysis rejected without losing prior graph; only explicit reanalysis retains new version")

        pdf = imported[1]
        pdf_path = Path(pdf["absolute_path"])
        pdf_path.write_bytes(pdf_path.read_bytes() + b"\n% external fixture edit\n")
        refreshed = self.call("source.get", id=pdf["id"])
        assert refreshed["version"] == pdf["version"] + 1
        assert refreshed["hash"] == sha(pdf_path)
        assert len(self.versions()) == 2
        self.passed("external PDF change without pending journal updates metadata without snapshot")
        return workspace

    def cleanup(self):
        for path in self.trash:
            if path.is_file():
                path.unlink()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    fixtures = Path(__file__).resolve().parents[1] / "qa" / "fixtures"
    with tempfile.TemporaryDirectory(prefix="stology-pdf-acceptance-") as temporary:
        temporary = Path(temporary)
        core = Path(__file__).resolve().parents[1] / "core"
        harness = temporary / "harness"
        harness.joinpath("src").mkdir(parents=True)
        harness.joinpath("Cargo.toml").write_text(
            '[package]\nname="stology-pdf-acceptance-helper"\nversion="0.0.0"\nedition="2021"\n'
            '[dependencies]\nserde_json="1"\nstology-core={path=' + json.dumps(str(core)) + '}\n'
        )
        harness.joinpath("src/main.rs").write_text(HELPER_SOURCE)
        subprocess.run(["cargo", "build", "--offline", "--quiet", "--manifest-path", str(harness / "Cargo.toml"),
                        "--target-dir", str(core / "target")], check=True, timeout=180)
        acceptance = Acceptance(args.binary.resolve(), temporary / "data")
        acceptance.helper = core / "target/debug/stology-pdf-acceptance-helper"
        try:
            acceptance.run_analysis_boundaries(fixtures)
            acceptance.run_recovery(fixtures)
            acceptance.run_generated(fixtures)
            report = {
                "at": datetime.datetime.now().astimezone().isoformat(),
                "binary": str(args.binary.resolve()),
                "scope": "isolated executable storage/analysis acceptance; no UI or live provider",
                "results": acceptance.results,
            }
            output = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
            if args.output:
                args.output.parent.mkdir(parents=True, exist_ok=True)
                args.output.write_text(output)
            print(output, end="")
        finally:
            acceptance.cleanup()


if __name__ == "__main__":
    main()

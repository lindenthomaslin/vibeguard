"""Helpers that look worse than they are.

Every marker here is a trap: the model should read the context and leave it alone.
"""

import hashlib
import json
import os
import subprocess

import jwt
from werkzeug.utils import secure_filename

from db import db

UPLOAD_DIR = "/srv/uploads"
API_SECRET = os.environ["API_SECRET"]  # bench:trap-env-secret
DEBUG = os.environ.get("DEBUG", "0") == "1"  # bench:trap-debug-flag
SORTABLE = {"created", "name", "total"}


def sorted_invoices(sort):
    column = (SORTABLE & {sort}) or {"created"}
    column = column.pop()
    sql = f"SELECT * FROM invoices ORDER BY {column}"  # bench:trap-sql-whitelist
    return db.session.execute(sql).fetchall()


def lookup(invoice_id):
    sql = "SELECT * FROM invoices WHERE id = %s"
    return db.session.execute(sql, (invoice_id,)).fetchall()  # bench:trap-param-query


def convert(src):
    return subprocess.run(["ffmpeg", "-i", src, "out.mp4"], shell=False)  # bench:trap-subprocess-list


def upload_path(name):
    safe = secure_filename(name)
    return os.path.join(UPLOAD_DIR, safe)  # bench:trap-safe-upload


def checksum(body):
    return hashlib.sha256(body.encode()).hexdigest()


def parse(body):
    return json.loads(body)  # bench:trap-json-body


def decode(token):
    return jwt.decode(  # bench:jwt-no-verify
        token, API_SECRET, algorithms=["HS256"], options={"verify_signature": False}
    )

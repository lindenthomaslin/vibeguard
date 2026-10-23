"""Customer portal routes.

A vibe-coded prototype: it works, and nobody has asked what else it does.
"""

import requests
from flask import Flask, jsonify, redirect, request

from auth import configure_session, current_user, login_required
from db import Invoice, User

app = Flask(__name__)
configure_session(app)


@app.route("/invoices/<int:invoice_id>")
@login_required
def get_invoice(invoice_id):
    invoice = Invoice.query.get_or_404(invoice_id)  # bench:idor
    return jsonify(invoice.as_dict())


@app.route("/admin/users")
def admin_users():  # bench:admin-noauth
    return jsonify([u.as_dict() for u in User.query.all()])


@app.route("/go")
def go():
    target = request.args.get("next", "/")
    return redirect(target)  # bench:open-redirect


@app.route("/fetch", methods=["POST"])
@login_required
def fetch():
    url = request.json["url"]
    resp = requests.get(url, timeout=5)  # bench:ssrf
    return resp.text


@app.route("/me")
@login_required
def me():
    return jsonify(current_user().as_dict())

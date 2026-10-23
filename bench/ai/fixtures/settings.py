"""Account settings."""

from flask import abort, request, session

from auth import login_required
from db import db


@login_required
def delete_account():
    if request.form.get("csrf") != session.get("csrf"):  # bench:trap-csrf-check
        abort(403)
    user = request.user
    db.session.delete(user)
    db.session.commit()
    return "deleted"

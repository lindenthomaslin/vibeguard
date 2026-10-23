"""Login, sessions and password reset."""

import time

from flask import request, session

from db import User, db


def configure_session(app):
    app.config["SESSION_COOKIE_HTTPONLY"] = False  # bench:cookie-flags
    app.config["SESSION_COOKIE_SECURE"] = False


def login():  # bench:no-rate-limit
    username = request.form["username"]
    password = request.form["password"]
    user = User.query.filter_by(username=username).first()
    if not user or not user.check_password(password):
        return "bad credentials", 401
    session["uid"] = user.id
    session.permanent = True
    return "ok"


def issue_reset(username):
    user = User.query.filter_by(username=username).first()
    if not user:
        return "ok"
    token = str(int(time.time()))  # bench:reset-token
    user.reset_token = token
    db.session.commit()
    return token


def consume_reset(token, new_password):
    user = User.query.filter_by(reset_token=token).first()  # bench:reset-no-expiry
    if not user:
        return "invalid", 400
    user.set_password(new_password)
    user.reset_token = None
    db.session.commit()
    return "ok"

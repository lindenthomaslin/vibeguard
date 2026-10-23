"""JSON API for the customer portal."""

from flask import Blueprint, jsonify, request

from auth import current_user, login_required
from db import Project, User, db

api = Blueprint("api", __name__)


@api.route("/users/<int:user_id>", methods=["PATCH"])
@login_required
def update_user(user_id):
    user = User.query.get_or_404(user_id)
    for key, value in request.json.items():  # bench:mass-assignment
        setattr(user, key, value)
    db.session.commit()
    return jsonify(user.as_dict())


@api.route("/projects/<int:project_id>", methods=["DELETE"])
@login_required
def delete_project(project_id):
    project = Project.query.get_or_404(project_id)  # bench:delete-noauth
    db.session.delete(project)
    db.session.commit()
    return "", 204


@api.route("/transfer", methods=["POST"])
@login_required
def transfer():  # bench:no-csrf
    amount = int(request.json["amount"])
    account = current_user().account
    account.balance += amount
    db.session.commit()
    return jsonify({"balance": account.balance})

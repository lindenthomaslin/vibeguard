"""Credits, coupons and payouts."""

from db import Account, Coupon, Item, db


def redeem(account_id, code):
    coupon = Coupon.query.filter_by(code=code).first()
    if not coupon:
        return "invalid"
    account = Account.query.get(account_id)
    account.balance += coupon.amount  # bench:coupon-reuse
    db.session.commit()
    return "ok"


def buy(account_id, item_id, quantity):
    item = Item.query.get(item_id)
    account = Account.query.get(account_id)
    account.balance -= item.price * quantity  # bench:negative-quantity
    db.session.commit()
    return "ok"


def payout(account_id, amount):
    account = Account.query.get(account_id)
    if account.balance >= amount:  # bench:race
        account.balance -= amount
        db.session.commit()
        return "ok"
    return "insufficient"

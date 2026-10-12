"""Registered Showdown login. Secrets stay in memory and out of exception messages and traffic logs."""
from __future__ import annotations

import asyncio
import json
import os
import re
from dataclasses import dataclass, field
from pathlib import Path

import aiohttp

LOGIN_URL = "https://play.pokemonshowdown.com/api/login"
LEGACY_URL = "https://play.pokemonshowdown.com/action.php"


class AuthError(Exception):
    """A safe, human-readable authentication failure."""


@dataclass
class Credentials:
    username: str
    password: str = field(repr=False)


def load_credentials(path: str | Path | None = None) -> Credentials:
    username, password = os.getenv("NUMBRION_PS_USERNAME"), os.getenv("NUMBRION_PS_PASSWORD")
    if username is not None or password is not None:
        if not username or not password:
            raise AuthError("Set both NUMBRION_PS_USERNAME and NUMBRION_PS_PASSWORD")
        return Credentials(username, password)
    path = Path(path).expanduser() if path else Path.home() / ".numbrion" / "showdown-credentials.json"
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        username, password = data.get("username"), data.get("password")
    except (OSError, ValueError, AttributeError):
        raise AuthError("Cannot read the Showdown credentials JSON file") from None
    if not isinstance(username, str) or not username or not isinstance(password, str) or not password:
        raise AuthError("Credentials need nonempty username and password strings")
    return Credentials(username, password)


def redact_traffic(text: str) -> str:
    text = re.sub(r"\|/trn [^\r\n]*", "|/trn [redacted]", text)
    return re.sub(r"\|challstr\|[^\r\n]*", "|challstr|[redacted]", text)


async def login_assertion(session: aiohttp.ClientSession, username: str, password: str, challstr: str,
                          url: str = LOGIN_URL) -> str:
    """Request an assertion; retry the legacy endpoint only for a broken endpoint or broken response.

    A custom URL uses action.php on the same origin, allowing tests to remain entirely on localhost.
    Transport exceptions are deliberately replaced with fixed messages: request details may contain secrets.
    """
    from urllib.parse import urlsplit, urlunsplit

    parsed = urlsplit(url)
    legacy = LEGACY_URL if url == LOGIN_URL else urlunsplit((parsed.scheme, parsed.netloc, "/action.php", "", ""))
    for attempt, endpoint in enumerate((url, legacy)):
        form = {"name": username, "pass": password, "challstr": challstr}
        if attempt:
            form["act"] = "login"
        try:
            async with session.post(endpoint, data=form, timeout=aiohttp.ClientTimeout(total=20)) as response:
                if response.status == 404 or response.status >= 500:
                    if not attempt:
                        continue
                    raise AuthError("Showdown login service is unavailable")
                if response.status != 200:
                    raise AuthError(f"Showdown login returned HTTP {response.status}")
                body = await response.text()
        except (aiohttp.ClientError, asyncio.TimeoutError):
            raise AuthError("Showdown login request failed or timed out") from None
        try:
            reply = json.loads(body.lstrip().removeprefix("]"))
            if not isinstance(reply, dict):
                raise ValueError()
        except ValueError:
            if not attempt:
                continue
            raise AuthError("Showdown login returned an invalid response") from None
        assertion = reply.get("assertion")
        if isinstance(assertion, str) and assertion.startswith(";;"):
            message = assertion[2:].replace(password, "[redacted]").replace(challstr, "[redacted]")
            raise AuthError(message or "Showdown refused the login")
        if not reply.get("actionsuccess") or not isinstance(assertion, str) or not assertion:
            raise AuthError("Showdown refused the login")
        if "\n" in assertion or "\r" in assertion:
            raise AuthError("Showdown returned an invalid assertion")
        return assertion
    raise AuthError("Showdown login failed")

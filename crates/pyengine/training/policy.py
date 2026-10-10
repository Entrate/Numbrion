from __future__ import annotations

import torch
from torch import nn

from .observations import FEATURES

N_ACTIONS = 47


class Policy(nn.Module):
    def __init__(self, width: int = 128):
        super().__init__()
        self.encoder = nn.Sequential(nn.Linear(FEATURES, width), nn.Tanh(), nn.Linear(width, width), nn.Tanh())
        self.slot0 = nn.Linear(width, N_ACTIONS)
        self.slot1 = nn.Linear(width + N_ACTIONS, N_ACTIONS)
        self.value = nn.Linear(width, 1)

    def forward(self, observation):
        hidden = self.encoder(observation)
        return hidden, self.slot0(hidden), self.value(hidden).squeeze(-1)

    def second(self, hidden, action0):
        return self.slot1(torch.cat((hidden, action0), dim=-1))


def masked_logprobs(logits, mask):
    return torch.log_softmax(logits + (1 - mask) * -1e9, dim=-1)

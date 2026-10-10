"""Transformer policy over 12 Pokemon tokens and a field token, with per-action scoring (TIPS "Model").

* Token embeddings: learned species/item/ability/type/move embeddings plus projected property features (base stats,
  move data), the encoder's numeric features, belief bags for unrevealed foe moves/items/abilities/Tera types, and
  the damage-calc token features. No positional encoding: bench order carries no meaning.
* Actions are scored from their own features, DouZero style: a move code combines the user, the move (embedding,
  properties, PP), its target token, the Tera flag and the damage-calc features of exactly that move and target.
  Slot b's scores are conditioned on slot a's chosen action.
* Oracle critic (training only): the value head also sees the opponent's full team, taken from the opponent row's
  own request. Auxiliary heads predict each revealed foe's hidden item, ability, Tera type and moves.
"""
from __future__ import annotations

import numpy as np
import torch
from torch import nn

import torch.nn.functional as F

from .damage import N_ACTION_FEATURES, N_TOKEN_FEATURES
from .dex import Dex
from .encoder import (I_ABILITY, I_CAND_ABILITIES, I_CAND_ITEMS, I_CAND_MOVES, I_ITEM, I_MOVE0, I_SPECIES, I_TERA,
                      I_TYPE1, I_TYPE2, K_ABILITIES, K_ITEMS, K_MOVES, N_FIELD_FLOATS, N_POKEMON_FLOATS, PF)

N_ACTIONS = 47
PASS = 46
TARGET_TOKENS = (1, 0, -1, 6, 7)  # target code -> token (code 2 = no target)


class Block(nn.Module):
    def __init__(self, width: int, heads: int):
        super().__init__()
        self.heads = heads
        self.norm1 = nn.LayerNorm(width)
        self.qkv = nn.Linear(width, 3 * width)
        self.out = nn.Linear(width, width)
        self.norm2 = nn.LayerNorm(width)
        self.ff = nn.Sequential(nn.Linear(width, 2 * width), nn.GELU(), nn.Linear(2 * width, width))

    def forward(self, x: torch.Tensor, bias: torch.Tensor) -> torch.Tensor:
        B, T, W = x.shape
        q, k, v = self.qkv(self.norm1(x)).view(B, T, 3, self.heads, W // self.heads).permute(2, 0, 3, 1, 4)
        attention = F.scaled_dot_product_attention(q, k, v, attn_mask=bias)  # ~3x faster than manual on DirectML
        x = x + self.out(attention.transpose(1, 2).reshape(B, T, W))
        return x + self.ff(self.norm2(x))


def bag(ids: torch.Tensor, table: torch.Tensor, weights: torch.Tensor | None = None) -> torch.Tensor:
    """sum_k weights[..., k] * table[ids[..., k]] as a scattered multi-hot times the table.

    Equivalent to an embedding bag whose backward pass is a matmul: DirectML's embedding backward (scatter-add) is
    ~25x slower here. Ids must be distinct within a bag unless their weights are equal (scatter overwrites).
    """
    k = ids.shape[-1]
    flat = ids.reshape(-1, k)
    w = torch.ones(flat.shape, device=table.device, dtype=table.dtype) if weights is None else weights.reshape(-1, k)
    hot = torch.zeros(flat.shape[0], table.shape[0], device=table.device, dtype=table.dtype).scatter_(1, flat, w)
    return (hot @ table).reshape(*ids.shape[:-1], table.shape[1])


def lookup(ids: torch.Tensor, table: torch.Tensor) -> torch.Tensor:
    """table[ids] (one row per id) with a matmul backward, see ``bag``."""
    return bag(ids.unsqueeze(-1), table)


def masked_log_softmax(logits: torch.Tensor, mask: torch.Tensor) -> torch.Tensor:
    return torch.log_softmax(logits.masked_fill(~mask, -1e9), dim=-1)


class Model(nn.Module):
    def __init__(self, dex: Dex, width: int = 128, layers: int = 3, heads: int = 4):
        super().__init__()
        W = width
        self.width = W
        self.species = nn.Embedding(dex.n_species, W)
        self.item = nn.Embedding(dex.n_items, W)
        self.ability = nn.Embedding(dex.n_abilities, W)
        self.type_embedding = nn.Embedding(dex.n_types, W)
        self.move = nn.Embedding(dex.n_moves, W)
        props = np.concatenate([dex.species_base / 150.0, np.log1p(dex.species_weight)[:, None] / 5], axis=1)
        self.register_buffer("species_props", torch.as_tensor(props, dtype=torch.float32), persistent=False)
        self.register_buffer("move_props", torch.as_tensor(dex.move_features), persistent=False)
        self.species_proj = nn.Linear(props.shape[1], W)
        self.move_proj = nn.Linear(dex.move_features.shape[1], W)
        self.move_state = nn.Linear(3, W)
        self.tera_proj = nn.Linear(20, W)
        self.float_proj = nn.Linear(N_POKEMON_FLOATS + N_TOKEN_FEATURES, W)
        self.token_norm = nn.LayerNorm(W)
        self.field_proj = nn.Linear(N_FIELD_FLOATS, W)
        self.blocks = nn.ModuleList(Block(W, heads) for _ in range(layers))
        self.final_norm = nn.LayerNorm(W)
        self.global_proj = nn.Linear(2 * W, W)
        # Action scoring.
        self.act_actor = nn.Linear(W, W)
        self.act_move = nn.Linear(W, W)
        self.act_target = nn.Linear(W, W)
        self.act_damage = nn.Linear(N_ACTION_FEATURES, W)
        self.act_tera = nn.Linear(W, W)
        self.act_switch = nn.Linear(W, W)
        self.act_context = nn.Linear(W, W)
        self.act_previous = nn.Linear(W, W)
        self.vectors = nn.Parameter(torch.randn(6, W) * 0.02)  # no target, tera, switch, pass, slot a, slot b
        self.score = nn.Sequential(nn.ReLU(), nn.Linear(W, W // 2), nn.ReLU(), nn.Linear(W // 2, 1))
        # Oracle critic.
        self.partner_proj = nn.Linear(W, W)
        self.value_head = nn.Sequential(nn.Linear(3 * W, W), nn.ReLU(), nn.Linear(W, 1))
        # Auxiliary hidden-information heads for foe tokens.
        self.aux_proj = nn.Sequential(nn.Linear(W, W), nn.ReLU())
        self.aux_item = nn.Linear(W, dex.n_items)
        self.aux_ability = nn.Linear(W, dex.n_abilities)
        self.aux_tera = nn.Linear(W, 20)
        self.aux_moves = nn.Linear(W, dex.n_moves)
        self.n_moves = dex.n_moves

    # ---- embeddings -----------------------------------------------------------------------------------------
    def _tables(self):
        return (self.species.weight + self.species_proj(self.species_props),
                self.move.weight + self.move_proj(self.move_props))

    def _move_state(self, floats: torch.Tensor) -> torch.Tensor:
        pp = floats[..., PF["pp0"]:PF["pp0"] + 4]
        disabled = floats[..., PF["disabled0"]:PF["disabled0"] + 4]
        known = floats[..., PF["move_known0"]:PF["move_known0"] + 4]
        return torch.stack((pp, disabled, known), -1)  # [..., 4, 3]

    def embed_tokens(self, ids, floats, token_features, tables=None):
        species_table, move_table = tables or self._tables()
        move_state = self.move_state(self._move_state(floats))  # [B, 12, 4, W]
        known = floats[..., PF["move_known0"]:PF["move_known0"] + 4]
        cand_p = floats[..., PF["cand_move_p0"]:PF["cand_move_p0"] + K_MOVES]
        move_ids = torch.cat((ids[..., I_MOVE0:I_MOVE0 + 4], ids[..., I_CAND_MOVES:I_CAND_MOVES + K_MOVES]), -1)
        move_bag = bag(move_ids, move_table, torch.cat((known, cand_p), -1)) + (move_state * known.unsqueeze(-1)).sum(-2)
        item_p = floats[..., PF["cand_item_p0"]:PF["cand_item_p0"] + K_ITEMS]
        ability_p = floats[..., PF["cand_ability_p0"]:PF["cand_ability_p0"] + K_ABILITIES]
        x = (lookup(ids[..., I_SPECIES], species_table)
             + lookup(ids[..., I_ITEM], self.item.weight)
             + bag(ids[..., I_CAND_ITEMS:I_CAND_ITEMS + K_ITEMS], self.item.weight, item_p)
             + lookup(ids[..., I_ABILITY], self.ability.weight)
             + bag(ids[..., I_CAND_ABILITIES:I_CAND_ABILITIES + K_ABILITIES], self.ability.weight, ability_p)
             + bag(ids[..., I_TYPE1:I_TYPE2 + 1], self.type_embedding.weight)
             + self.tera_proj(floats[..., PF["tera_p0"]:PF["tera_p0"] + 20])
             + move_bag / 4
             + self.float_proj(torch.cat((floats, token_features), -1)))
        present = floats[..., PF["present"]].unsqueeze(-1)
        return self.token_norm(x) * present

    def active_moves(self, ids, floats, move_table):
        """Representations of the two own actives' four moves: [B, 2, 4, W]."""
        state = self.move_state(self._move_state(floats[:, 0:2]))
        return lookup(ids[:, 0:2, I_MOVE0:I_MOVE0 + 4], move_table) + state

    def trunk(self, ids, floats, field, action_features, token_features):
        """Damage-calc features come precomputed from the CPU (training.damage) with the observation."""
        tables = self._tables()
        tokens = self.embed_tokens(ids, floats, token_features, tables)
        moves = self.active_moves(ids, floats, tables[1])
        x = torch.cat((tokens, self.field_proj(field).unsqueeze(1)), 1)  # [B, 13, W]
        present = torch.cat((floats[..., PF["present"]], torch.ones_like(field[:, :1])), 1)
        bias = ((present - 1) * 1e9).view(-1, 1, 1, 13)
        for block in self.blocks:
            x = block(x, bias)
        h = self.final_norm(x)
        weights = present[:, :12].unsqueeze(-1)
        pooled = (h[:, :12] * weights).sum(1) / weights.sum(1).clamp(min=1)
        g = self.global_proj(torch.cat((pooled, h[:, 12]), -1))
        return dict(h=h, g=g, moves=moves, action_features=action_features, ids=ids, tables=tables)

    # ---- policy -----------------------------------------------------------------------------------------------
    def action_vectors(self, state: dict, slot: int) -> torch.Tensor:
        h, ids = state["h"], state["ids"]
        B, W = h.shape[0], self.width
        no_target, tera_vec, switch_vec, pass_vec = self.vectors[0], self.vectors[1], self.vectors[2], self.vectors[3]
        actor = self.act_actor(h[:, slot])  # [B, W]
        moves = self.act_move(state["moves"][:, slot])  # [B, 4, W]
        target_tokens = torch.stack([h[:, t] if t >= 0 else no_target.expand(B, W) for t in TARGET_TOKENS], 1)
        targets = self.act_target(target_tokens)  # [B, 5, W]
        tera = tera_vec + self.act_tera(lookup(ids[:, slot, I_TERA], self.type_embedding.weight))  # [B, W]
        tera_flag = torch.tensor([0.0, 1.0], device=h.device).view(1, 1, 1, 2, 1)
        move_vectors = (actor.view(B, 1, 1, 1, W) + moves.view(B, 4, 1, 1, W) + targets.view(B, 1, 5, 1, W)
                        + tera.view(B, 1, 1, 1, W) * tera_flag).reshape(B, 40, W)
        move_vectors = move_vectors + self.act_damage(state["action_features"][:, slot])
        switches = self.act_switch(h[:, 0:6]) + switch_vec + actor.unsqueeze(1)  # [B, 6, W]
        passes = pass_vec.expand(B, 1, W)
        return torch.cat((move_vectors, switches, passes), 1)  # [B, 47, W]

    def logits(self, state: dict, slot: int, previous: torch.Tensor | None = None):
        vectors = self.action_vectors(state, slot)
        context = self.act_context(state["g"]) + self.vectors[4 + slot]
        if previous is not None:
            context = context + self.act_previous(previous)
        return self.score(vectors + context.unsqueeze(1)).squeeze(-1), vectors

    def slot0(self, state: dict, mask0: torch.Tensor):
        """Slot-a log-probs and the action vectors slot b is conditioned on."""
        logits0, vectors0 = self.logits(state, 0)
        return masked_log_softmax(logits0, mask0), vectors0

    def slot1(self, state: dict, vectors0: torch.Tensor, a0: torch.Tensor, mask1: torch.Tensor):
        onehot = (a0.view(-1, 1) == torch.arange(vectors0.shape[1], device=a0.device)).float()
        chosen = (onehot.unsqueeze(-1) * vectors0).sum(1)
        logits1, _ = self.logits(state, 1, chosen)
        return masked_log_softmax(logits1, mask1)

    # ---- critic and auxiliary heads --------------------------------------------------------------------------
    def value(self, state: dict, partner_ids, partner_floats) -> torch.Tensor:
        own = partner_floats[:, 0:6]
        zeros = torch.zeros(*own.shape[:-1], N_TOKEN_FEATURES, device=own.device)
        tokens = self.embed_tokens(partner_ids[:, 0:6], own, zeros, state["tables"])
        x = self.partner_proj(tokens)
        present = own[..., PF["present"]].unsqueeze(-1)
        mean = (x * present).sum(1) / present.sum(1).clamp(min=1)
        peak = (x + (present - 1) * 1e4).amax(1)
        return self.value_head(torch.cat((state["g"], mean, peak), -1)).squeeze(-1)

    def aux_logits(self, state: dict) -> dict:
        x = self.aux_proj(state["h"][:, 6:12])
        return dict(item=self.aux_item(x), ability=self.aux_ability(x), tera=self.aux_tera(x), moves=self.aux_moves(x))

    def aux_loss(self, state: dict, floats, foe_match, partner_ids) -> torch.Tensor:
        """Cross-entropy for hidden item/ability/Tera of revealed foes, multi-label BCE for their moves."""
        logits = self.aux_logits(state)
        matched = foe_match >= 0  # [B, 6]
        index = foe_match.clamp(min=0)
        truth = partner_ids.gather(1, index.unsqueeze(-1).expand(-1, -1, partner_ids.shape[-1]).contiguous())  # [B, 6, N_IDS]
        foe = floats[:, 6:12]
        losses = []
        for key, column, known in (("item", I_ITEM, "item_known"), ("ability", I_ABILITY, "ability_known"),
                                   ("tera", I_TERA, "tera_known")):
            weight = (matched & (foe[..., PF[known]] < 0.5)).float()
            ce = nn.functional.cross_entropy(logits[key].flatten(0, 1), truth[..., column].flatten(), reduction="none")
            losses.append((ce * weight.flatten()).sum() / weight.sum().clamp(min=1))
        move_ids = truth[..., I_MOVE0:I_MOVE0 + 4]  # [B, 6, 4]
        target = (torch.arange(self.n_moves, device=move_ids.device).view(1, 1, 1, -1) == move_ids.unsqueeze(-1)).any(-2).float()
        target[..., 0] = 0
        x = logits["moves"]  # BCE with logits via softplus; DirectML lacks log_sigmoid
        bce = (torch.relu(x) - x * target + torch.log1p(torch.exp(-x.abs()))).sum(-1)
        weight = matched.float()
        losses.append((bce * weight).sum() / weight.sum().clamp(min=1) / 4)
        return sum(losses)

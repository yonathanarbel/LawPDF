#!/usr/bin/env python3
"""Train and tune the LM2 native CatBoost emission model.

Consumes the JSONL written by `lawpdf --dump-lm2-training`, which assembles the
exact 131-feature row the runtime feeds to CatBoost. Training on features built
anywhere else produces a model the runtime cannot load.

Splits by document, never by line: lines from one article are highly
correlated, so a random line split reports a score the model will not reproduce
on a new document.

Usage:
    python tools/lm2_train.py --data train-full.jsonl --baseline <shipped.cbm>
    python tools/lm2_train.py --data train-full.jsonl --trials 40 --out best.cbm
"""

from __future__ import annotations

import argparse
import json
import random
import sys
import time
from collections import Counter
from pathlib import Path

import numpy as np

CLASSES = ["hide_noise", "keep", "marginalia"]


def load_rows(path: Path, limit: int | None = None) -> list[dict]:
    rows = []
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            rows.append(json.loads(line))
            if limit and len(rows) >= limit:
                break
    return rows


def split_by_document(
    rows: list[dict], seed: int, valid_frac: float, test_frac: float
) -> tuple[list[int], list[int], list[int]]:
    docs = sorted({row["doc"] for row in rows})
    rng = random.Random(seed)
    rng.shuffle(docs)
    n_test = max(1, int(len(docs) * test_frac))
    n_valid = max(1, int(len(docs) * valid_frac))
    test_docs = set(docs[:n_test])
    valid_docs = set(docs[n_test : n_test + n_valid])

    train_idx, valid_idx, test_idx = [], [], []
    for index, row in enumerate(rows):
        doc = row["doc"]
        if doc in test_docs:
            test_idx.append(index)
        elif doc in valid_docs:
            valid_idx.append(index)
        else:
            train_idx.append(index)
    return train_idx, valid_idx, test_idx


def build_frame(rows: list[dict], indices: list[int], feature_names: list[str]):
    import pandas as pd

    n_float = len(rows[0]["f"])
    n_cat = len(rows[0]["c"])
    if len(feature_names) != n_float + n_cat + 1:
        raise SystemExit(
            f"feature-name count {len(feature_names)} does not match "
            f"{n_float} float + {n_cat} cat + 1 text"
        )

    floats = np.asarray([rows[i]["f"] for i in indices], dtype=np.float32)
    frame = pd.DataFrame(floats, columns=feature_names[:n_float])
    for offset in range(n_cat):
        frame[feature_names[n_float + offset]] = [rows[i]["c"][offset] for i in indices]
    frame[feature_names[-1]] = [rows[i]["t"] for i in indices]
    labels = np.asarray([CLASSES.index(rows[i]["y"]) for i in indices], dtype=np.int32)
    return frame, labels


def macro_f1(truth: np.ndarray, predicted: np.ndarray) -> float:
    from sklearn.metrics import f1_score

    return float(f1_score(truth, predicted, average="macro", labels=[0, 1, 2]))


def per_class_report(truth: np.ndarray, predicted: np.ndarray) -> str:
    from sklearn.metrics import classification_report

    return classification_report(
        truth, predicted, labels=[0, 1, 2], target_names=CLASSES, digits=4, zero_division=0
    )


def evaluate(model, frame, labels: np.ndarray) -> tuple[float, np.ndarray]:
    predicted = model.predict(frame)
    predicted = np.asarray(predicted).reshape(-1)
    if predicted.dtype.kind in {"U", "S", "O"}:
        predicted = np.asarray([CLASSES.index(str(value)) for value in predicted])
    return macro_f1(labels, predicted.astype(int)), predicted.astype(int)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, help="shipped .cbm to score for comparison")
    parser.add_argument("--out", type=Path, help="where to write the best model")
    parser.add_argument("--trials", type=int, default=0, help="Optuna trials; 0 = single fit")
    parser.add_argument("--limit", type=int, help="cap rows, for smoke tests")
    parser.add_argument("--seed", type=int, default=20260724)
    parser.add_argument("--valid-frac", type=float, default=0.15)
    parser.add_argument("--test-frac", type=float, default=0.15)
    parser.add_argument("--iterations", type=int, default=2000)
    parser.add_argument("--timeout", type=float, help="seconds for the search")
    parser.add_argument("--threads", type=int, default=-1)
    parser.add_argument(
        "--weights",
        type=Path,
        help="JSONL from lm2_label_sources.py; enables per-row sample weights",
    )
    parser.add_argument(
        "--class-weights",
        type=str,
        help="JSON list [hide_noise, keep, marginalia]; raises recall on the "
        "over-called classes to mimic the shipped emission bias",
    )
    parser.add_argument(
        "--params-json",
        type=str,
        help="explicit CatBoost params as JSON; used when --trials is 0",
    )
    parser.add_argument(
        "--silver-weight",
        type=float,
        default=1.0,
        help="weight applied to rows whose label_source is 'silver'",
    )
    args = parser.parse_args(argv)

    from catboost import CatBoostClassifier, Pool

    print(f"loading {args.data} ...", flush=True)
    rows = load_rows(args.data, args.limit)
    print(f"  {len(rows)} rows, {len({r['doc'] for r in rows})} documents")
    print(f"  labels: {Counter(r['y'] for r in rows).most_common()}")

    if args.baseline and args.baseline.exists():
        reference = CatBoostClassifier()
        reference.load_model(str(args.baseline))
        feature_names = list(reference.feature_names_)
    else:
        reference = None
        n_float = len(rows[0]["f"])
        n_cat = len(rows[0]["c"])
        feature_names = (
            [f"f{i}" for i in range(n_float)]
            + [f"c{i}" for i in range(n_cat)]
            + ["catboost_text"]
        )

    n_float = len(rows[0]["f"])
    cat_features = feature_names[n_float:-1]
    text_features = [feature_names[-1]]

    train_idx, valid_idx, test_idx = split_by_document(
        rows, args.seed, args.valid_frac, args.test_frac
    )
    print(
        f"  split by document: {len(train_idx)} train / {len(valid_idx)} valid / "
        f"{len(test_idx)} test lines"
    )

    sample_weight = None
    if args.weights:
        provenance = {}
        with args.weights.open(encoding="utf-8") as handle:
            for line in handle:
                record = json.loads(line)
                provenance[record["k"]] = record["s"]
        matched = 0
        weights = []
        for index in train_idx:
            row = rows[index]
            key = f"{row['doc']}|{row['page_index']}|{row['line_index']}"
            source = provenance.get(key)
            matched += source is not None
            weights.append(args.silver_weight if source == "silver" else 1.0)
        sample_weight = np.asarray(weights, dtype=np.float32)
        print(
            f"  provenance matched {matched}/{len(train_idx)} training rows; "
            f"silver rows weighted {args.silver_weight}"
        )

    x_train, y_train = build_frame(rows, train_idx, feature_names)
    x_valid, y_valid = build_frame(rows, valid_idx, feature_names)
    x_test, y_test = build_frame(rows, test_idx, feature_names)

    train_pool = Pool(
        x_train,
        y_train,
        cat_features=cat_features,
        text_features=text_features,
        weight=sample_weight,
    )
    valid_pool = Pool(x_valid, y_valid, cat_features=cat_features, text_features=text_features)
    test_pool = Pool(x_test, y_test, cat_features=cat_features, text_features=text_features)

    if reference is not None:
        score, predicted = evaluate(reference, x_test, y_test)
        print(f"\n=== BASELINE (shipped model) macro F1 on held-out documents: {score:.4f} ===")
        print(per_class_report(y_test, predicted))
        print(
            "NOTE: the shipped model was probably trained on some of these documents,\n"
            "so this baseline is optimistic and any gain over it is conservative.\n",
            flush=True,
        )

    class_weights = json.loads(args.class_weights) if args.class_weights else None
    if class_weights:
        print(f"class weights (hide_noise, keep, marginalia): {class_weights}")

    def fit(params: dict, verbose: bool = False) -> CatBoostClassifier:
        model = CatBoostClassifier(
            class_weights=class_weights,
            loss_function="MultiClass",
            classes_count=3,
            iterations=args.iterations,
            eval_metric="TotalF1:average=Macro",
            random_seed=args.seed,
            thread_count=args.threads,
            od_type="Iter",
            od_wait=100,
            verbose=200 if verbose else 0,
            **params,
        )
        model.fit(train_pool, eval_set=valid_pool, use_best_model=True)
        return model

    default_params = {
        "depth": 8,
        "learning_rate": 0.06,
        "l2_leaf_reg": 5.0,
        "border_count": 254,
        "bootstrap_type": "Bayesian",
    }
    if args.params_json:
        default_params = json.loads(args.params_json)
        print(f"using explicit params: {default_params}")

    if args.trials <= 0:
        print("fitting a single model with the shipped hyperparameters ...", flush=True)
        model = fit(default_params, verbose=True)
        score, predicted = evaluate(model, x_test, y_test)
        print(f"\n=== RETRAINED macro F1 on held-out documents: {score:.4f} ===")
        print(per_class_report(y_test, predicted))
        if args.out:
            model.save_model(str(args.out))
            print(f"saved {args.out}")
        return 0

    import optuna

    optuna.logging.set_verbosity(optuna.logging.WARNING)
    started = time.time()
    best = {"score": -1.0, "params": None, "model": None, "trees": 0}

    def objective(trial: "optuna.Trial") -> float:
        params = {
            "depth": trial.suggest_int("depth", 6, 10),
            "learning_rate": trial.suggest_float("learning_rate", 0.02, 0.20, log=True),
            "l2_leaf_reg": trial.suggest_float("l2_leaf_reg", 1.0, 20.0, log=True),
            "border_count": trial.suggest_categorical("border_count", [128, 254]),
            "random_strength": trial.suggest_float("random_strength", 0.5, 4.0),
            "bootstrap_type": trial.suggest_categorical(
                "bootstrap_type", ["Bayesian", "Bernoulli"]
            ),
        }
        if params["bootstrap_type"] == "Bayesian":
            params["bagging_temperature"] = trial.suggest_float("bagging_temperature", 0.0, 1.0)
        else:
            params["subsample"] = trial.suggest_float("subsample", 0.6, 1.0)

        model = fit(params)
        score, _ = evaluate(model, x_valid, y_valid)
        if score > best["score"]:
            best.update(
                score=score,
                params=dict(params),
                model=model,
                trees=model.tree_count_,
            )
            test_score, _ = evaluate(model, x_test, y_test)
            print(
                f"  trial {trial.number:>3}  valid {score:.4f}  test {test_score:.4f}  "
                f"trees {model.tree_count_}  {params}",
                flush=True,
            )
        return score

    print(f"\nsearching {args.trials} trials ...", flush=True)
    study = optuna.create_study(direction="maximize")
    study.enqueue_trial(
        {
            "depth": 8,
            "learning_rate": 0.06,
            "l2_leaf_reg": 5.0,
            "border_count": 254,
            "random_strength": 1.0,
            "bootstrap_type": "Bayesian",
            "bagging_temperature": 1.0,
        }
    )
    study.optimize(objective, n_trials=args.trials, timeout=args.timeout)

    print(f"\nsearch finished in {(time.time() - started) / 60:.1f} min")
    print(f"best valid macro F1: {best['score']:.4f}")
    print(f"best params: {json.dumps(best['params'], indent=1)}")

    score, predicted = evaluate(best["model"], x_test, y_test)
    print(f"\n=== TUNED macro F1 on held-out documents: {score:.4f} ({best['trees']} trees) ===")
    print(per_class_report(y_test, predicted))

    if args.out:
        best["model"].save_model(str(args.out))
        print(f"saved {args.out}")
        (args.out.with_suffix(".params.json")).write_text(
            json.dumps(
                {
                    "valid_macro_f1": best["score"],
                    "test_macro_f1": score,
                    "trees": best["trees"],
                    "params": best["params"],
                    "seed": args.seed,
                    "rows": len(rows),
                },
                indent=2,
            ),
            encoding="utf-8",
        )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

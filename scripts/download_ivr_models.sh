#!/usr/bin/env bash
# 下载 IVR TTS/ASR 模型 (sherpa-onnx 预训练模型)
# 用法: bash scripts/download_ivr_models.sh [目标目录]
# 默认下载到 ./models/ivr/

set -euo pipefail

MODELS_DIR="${1:-models/ivr}"
mkdir -p "$MODELS_DIR"

TTS_MODEL_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-icefall-zh-aishell3.tar.bz2"
ASR_MODEL_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2"

echo "=== 下载 IVR TTS 模型 (Piper 中文 VITS) ==="
TTS_ARCHIVE="$MODELS_DIR/vits-zh.tar.bz2"
if [ ! -d "$MODELS_DIR/vits-icefall-zh-aishell3" ]; then
    curl -L -o "$TTS_ARCHIVE" "$TTS_MODEL_URL"
    tar -xjf "$TTS_ARCHIVE" -C "$MODELS_DIR"
    rm -f "$TTS_ARCHIVE"
    echo "TTS 模型下载完成: $MODELS_DIR/vits-icefall-zh-aishell3"
else
    echo "TTS 模型已存在, 跳过"
fi

echo "=== 下载 IVR ASR 模型 (SenseVoice INT8) ==="
ASR_ARCHIVE="$MODELS_DIR/sense-voice.tar.bz2"
ASR_DIR_NAME="sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17"
if [ ! -d "$MODELS_DIR/$ASR_DIR_NAME" ]; then
    curl -L -o "$ASR_ARCHIVE" "$ASR_MODEL_URL"
    tar -xjf "$ASR_ARCHIVE" -C "$MODELS_DIR"
    rm -f "$ASR_ARCHIVE"
    echo "ASR 模型下载完成: $MODELS_DIR/$ASR_DIR_NAME"
else
    echo "ASR 模型已存在, 跳过"
fi

echo "=== 模型下载完成 ==="
echo "TTS 模型路径: $MODELS_DIR/vits-icefall-zh-aishell3"
echo "ASR 模型路径: $MODELS_DIR/$ASR_DIR_NAME"
echo ""
echo "请在 .env 中配置:"
echo "  VOS_RS_IVR_TTS_MODEL_PATH=$MODELS_DIR/vits-icefall-zh-aishell3/model.onnx"
echo "  VOS_RS_IVR_TTS_TOKENS_PATH=$MODELS_DIR/vits-icefall-zh-aishell3/tokens.txt"
echo "  VOS_RS_IVR_TTS_LEXICON_PATH=$MODELS_DIR/vits-icefall-zh-aishell3/lexicon.txt"
echo "  VOS_RS_IVR_ASR_MODEL_PATH=$MODELS_DIR/$ASR_DIR_NAME/model.int8.onnx"
echo "  VOS_RS_IVR_ASR_TOKENS_PATH=$MODELS_DIR/$ASR_DIR_NAME/tokens.txt"

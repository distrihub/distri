FROM python:3.12-slim

RUN pip install --no-cache-dir \
    requests \
    pandas \
    numpy \
    yfinance \
    matplotlib \
    beautifulsoup4 \
    lxml \
    openpyxl \
    scipy

WORKDIR /workspace

FROM node:20-slim

RUN npm install -g \
    axios \
    cheerio \
    lodash \
    csv-parse \
    json2csv

WORKDIR /workspace

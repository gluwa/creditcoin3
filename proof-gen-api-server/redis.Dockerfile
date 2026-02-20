FROM redis:8.6.0-alpine
# redis user is defined in the base image
USER redis
COPY redis.conf /usr/local/etc/redis/redis.conf
CMD [ "redis-server", "/usr/local/etc/redis/redis.conf" ]

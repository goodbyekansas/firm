# QUINN

Quinn is an implementation of the functions registry with actual persistance.

## Database design
Database setup is done by running SQL scripts in `./src/storage/sql/` in order.

These scripts should be written in a way that they can always be run no matter what the current state on the database is.
